use crate::error::{ExecutionError, GitError};
use crate::types::RepositoryInfo;
use secrecy::SecretString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use tokio::fs;
use tokio::time::timeout;
use tracing::{error, info, instrument};

#[derive(Debug, Clone)]
pub struct ServiceAuth {
    pub token_type: TokenType,
    pub token: SecretString,
    pub expiry: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone)]
pub enum TokenType {
    Bearer,
    SSH { key_path: PathBuf },
    BasicAuth { username: String },
}

pub struct RepositoryCloner {
    workspace_root: PathBuf,
    git_timeout: Duration,
    clone_depth: Option<u32>,
    lfs_enabled: bool,
}

impl RepositoryCloner {
    pub fn new(
        workspace_root: PathBuf,
        git_timeout: Duration,
        clone_depth: Option<u32>,
        lfs_enabled: bool,
    ) -> Self {
        Self {
            workspace_root,
            git_timeout,
            clone_depth,
            lfs_enabled,
        }
    }

    #[instrument(skip(self, auth), fields(repo = %repo.name))]
    pub async fn clone(
        &self,
        job_id: uuid::Uuid,
        repo: &RepositoryInfo,
        auth: &ServiceAuth,
    ) -> Result<PathBuf, ExecutionError> {
        let workspace = self.workspace_root.join(job_id.to_string());

        // Clean up existing workspace if it exists (from previous failed runs)
        if workspace.exists() {
            info!("Removing existing workspace at {}", workspace.display());
            fs::remove_dir_all(&workspace)
                .await
                .map_err(ExecutionError::IoError)?;
        }

        // Ensure parent directory exists (git clone will create the workspace directory itself)
        fs::create_dir_all(&self.workspace_root)
            .await
            .map_err(ExecutionError::IoError)?;

        // Clone repository (git clone will create the workspace directory)
        self.perform_clone(&workspace, repo, auth).await?;

        // Set permissions after clone
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o700);
            fs::set_permissions(&workspace, perms)
                .await
                .map_err(ExecutionError::IoError)?;
        }

        info!("Created workspace at {}", workspace.display());

        // Checkout specific commit
        self.checkout_commit(&workspace, &repo.commit_sha).await?;

        // Initialize submodules if present
        if workspace.join(".gitmodules").exists() {
            self.init_submodules(&workspace, auth).await?;
        }

        // Pull LFS objects if enabled
        if self.lfs_enabled && workspace.join(".gitattributes").exists() {
            self.pull_lfs(&workspace, auth).await?;
        }

        // Validate commit SHA
        self.validate_commit(&workspace, &repo.commit_sha).await?;

        Ok(workspace)
    }

    async fn perform_clone(
        &self,
        workspace: &Path,
        repo: &RepositoryInfo,
        auth: &ServiceAuth,
    ) -> Result<(), ExecutionError> {
        let mut cmd = Command::new("git");
        cmd.arg("clone");

        if let Some(depth) = self.clone_depth {
            cmd.arg("--depth").arg(depth.to_string());
        }

        cmd.arg("--single-branch")
            .arg("--branch")
            .arg(&repo.ref_name)
            .arg(&repo.clone_url)
            .arg(workspace)
            .current_dir(&self.workspace_root);

        // Set up authentication
        self.setup_auth(&mut cmd, auth)?;

        info!("Cloning repository: {}", repo.clone_url);

        let output = match timeout(self.git_timeout, async {
            tokio::task::spawn_blocking(move || cmd.output())
                .await
                .map_err(|e| {
                    std::io::Error::other(
                        format!("Task join error: {}", e),
                    )
                })
        })
        .await
        {
            Ok(Ok(output)) => output,
            Ok(Err(e)) => return Err(ExecutionError::CloneError(GitError::IoError(e))),
            Err(_) => return Err(ExecutionError::CloneError(GitError::Timeout)),
        };

        let output = output.map_err(|e| ExecutionError::CloneError(GitError::IoError(e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("Authentication") || stderr.contains("Permission denied") {
                return Err(ExecutionError::CloneError(GitError::AuthFailed));
            }
            return Err(ExecutionError::CloneError(GitError::CommandFailed(
                format!("git clone failed: {}", stderr),
            )));
        }

        Ok(())
    }

    async fn checkout_commit(
        &self,
        workspace: &Path,
        commit_sha: &str,
    ) -> Result<(), ExecutionError> {
        let mut cmd = Command::new("git");
        cmd.arg("checkout").arg(commit_sha).current_dir(workspace);

        let output = match timeout(self.git_timeout, async {
            tokio::task::spawn_blocking(move || cmd.output())
                .await
                .map_err(|e| {
                    std::io::Error::other(
                        format!("Task join error: {}", e),
                    )
                })
        })
        .await
        {
            Ok(Ok(output)) => output,
            Ok(Err(e)) => return Err(ExecutionError::CloneError(GitError::IoError(e))),
            Err(_) => return Err(ExecutionError::CloneError(GitError::Timeout)),
        };

        let output = output.map_err(|e| ExecutionError::CloneError(GitError::IoError(e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ExecutionError::CloneError(GitError::CommandFailed(
                format!("git checkout failed: {}", stderr),
            )));
        }

        Ok(())
    }

    async fn init_submodules(
        &self,
        workspace: &Path,
        auth: &ServiceAuth,
    ) -> Result<(), ExecutionError> {
        let mut cmd = Command::new("git");
        cmd.arg("submodule")
            .arg("update")
            .arg("--init")
            .arg("--recursive")
            .current_dir(workspace);

        self.setup_auth(&mut cmd, auth)?;

        let output = match timeout(self.git_timeout, async {
            tokio::task::spawn_blocking(move || cmd.output())
                .await
                .map_err(|e| {
                    std::io::Error::other(
                        format!("Task join error: {}", e),
                    )
                })
        })
        .await
        {
            Ok(Ok(output)) => output,
            Ok(Err(e)) => return Err(ExecutionError::CloneError(GitError::IoError(e))),
            Err(_) => return Err(ExecutionError::CloneError(GitError::Timeout)),
        };

        let output = output.map_err(|e| ExecutionError::CloneError(GitError::IoError(e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            error!("Submodule initialization failed: {}", stderr);
            // Don't fail the job for submodule errors
        }

        Ok(())
    }

    async fn pull_lfs(&self, workspace: &Path, auth: &ServiceAuth) -> Result<(), ExecutionError> {
        let mut cmd = Command::new("git");
        cmd.arg("lfs").arg("pull").current_dir(workspace);

        self.setup_auth(&mut cmd, auth)?;

        let output = match timeout(self.git_timeout, async {
            tokio::task::spawn_blocking(move || cmd.output())
                .await
                .map_err(|e| {
                    std::io::Error::other(
                        format!("Task join error: {}", e),
                    )
                })
        })
        .await
        {
            Ok(Ok(output)) => output,
            Ok(Err(e)) => return Err(ExecutionError::CloneError(GitError::IoError(e))),
            Err(_) => return Err(ExecutionError::CloneError(GitError::Timeout)),
        };

        let output = output.map_err(|e| ExecutionError::CloneError(GitError::IoError(e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            error!("LFS pull failed: {}", stderr);
            // Don't fail the job for LFS errors
        }

        Ok(())
    }

    async fn validate_commit(
        &self,
        workspace: &Path,
        expected_sha: &str,
    ) -> Result<(), ExecutionError> {
        let mut cmd = Command::new("git");
        cmd.arg("rev-parse").arg("HEAD").current_dir(workspace);

        let output = match timeout(Duration::from_secs(10), async {
            tokio::task::spawn_blocking(move || cmd.output())
                .await
                .map_err(|e| {
                    std::io::Error::other(
                        format!("Task join error: {}", e),
                    )
                })
        })
        .await
        {
            Ok(Ok(output)) => output,
            Ok(Err(e)) => return Err(ExecutionError::CloneError(GitError::IoError(e))),
            Err(_) => return Err(ExecutionError::CloneError(GitError::Timeout)),
        };

        let output = output.map_err(|e| ExecutionError::CloneError(GitError::IoError(e)))?;

        if !output.status.success() {
            return Err(ExecutionError::CloneError(GitError::CommandFailed(
                "Failed to get current commit SHA".to_string(),
            )));
        }

        let actual_sha = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if actual_sha != expected_sha {
            return Err(ExecutionError::CloneError(GitError::CommandFailed(
                format!(
                    "Commit SHA mismatch: expected {}, got {}",
                    expected_sha, actual_sha
                ),
            )));
        }

        Ok(())
    }

    fn setup_auth(&self, cmd: &mut Command, auth: &ServiceAuth) -> Result<(), ExecutionError> {
        match &auth.token_type {
            TokenType::Bearer => {
                cmd.env("GIT_ASKPASS", "echo")
                    .env("GIT_TERMINAL_PROMPT", "0");
                // Note: In production, use a credential helper
                // For now, we'll rely on URL-based auth or SSH
            }
            TokenType::SSH { key_path } => {
                cmd.env(
                    "GIT_SSH_COMMAND",
                    format!("ssh -i {} -o StrictHostKeyChecking=no", key_path.display()),
                );
            }
            TokenType::BasicAuth { username: _ } => {
                cmd.env("GIT_ASKPASS", "echo")
                    .env("GIT_TERMINAL_PROMPT", "0");
                // URL format: https://username:token@host/path
            }
        }
        Ok(())
    }
}
