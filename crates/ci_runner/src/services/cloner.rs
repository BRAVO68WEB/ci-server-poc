use crate::models::error::{ExecutionError, GitError};
use crate::models::types::RepositoryInfo;
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
        info!("[CLONE] ===== Starting clone for job_id: {}, repo: {} =====", job_id, repo.name);
        let workspace = self.workspace_root.join(job_id.to_string());
        info!("[CLONE] Workspace path: {}", workspace.display());
        info!("[CLONE] Workspace root: {}", self.workspace_root.display());
        info!("[CLONE] Repository URL: {}", repo.clone_url);
        info!("[CLONE] Branch/Ref: {}", repo.ref_name);
        info!("[CLONE] Commit SHA: {}", repo.commit_sha);

        // Clean up existing workspace if it exists (from previous failed runs)
        if workspace.exists() {
            info!("[CLONE] Removing existing workspace at {}", workspace.display());
            fs::remove_dir_all(&workspace)
                .await
                .map_err(ExecutionError::IoError)?;
            info!("[CLONE] Existing workspace removed");
        } else {
            info!("[CLONE] Workspace does not exist, will create new");
        }

        // Ensure parent directory exists
        info!("[CLONE] Ensuring workspace root exists: {}", self.workspace_root.display());
        fs::create_dir_all(&self.workspace_root)
            .await
            .map_err(ExecutionError::IoError)?;
        info!("[CLONE] Workspace root exists");

        // Clone repository directly into workspace directory
        // perform_clone will create the workspace directory and clone into it
        info!("[CLONE] Calling perform_clone");
        self.perform_clone(&workspace, repo, auth).await?;
        info!("[CLONE] perform_clone completed successfully");

        // Set permissions after clone
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o700);
            fs::set_permissions(&workspace, perms)
                .await
                .map_err(ExecutionError::IoError)?;
        }

        info!("[CLONE] Created workspace at {}", workspace.display());

        // Checkout specific commit
        info!("[CLONE] Checking out commit: {}", repo.commit_sha);
        self.checkout_commit(&workspace, &repo.commit_sha).await?;
        info!("[CLONE] Commit checkout completed");

        // Initialize submodules if present
        if workspace.join(".gitmodules").exists() {
            self.init_submodules(&workspace, auth).await?;
        }

        // Pull LFS objects if enabled
        if self.lfs_enabled && workspace.join(".gitattributes").exists() {
            self.pull_lfs(&workspace, auth).await?;
        }

        // Validate commit SHA
        info!("[CLONE] Validating commit SHA: {}", repo.commit_sha);
        self.validate_commit(&workspace, &repo.commit_sha).await?;
        info!("[CLONE] Commit validation completed");

        // Final verification - list workspace contents
        info!("[CLONE] Final verification - listing workspace contents");
        let mut final_check = fs::read_dir(&workspace)
            .await
            .map_err(ExecutionError::IoError)?;
        let mut final_contents = Vec::new();
        while let Some(entry) = final_check.next_entry().await.map_err(ExecutionError::IoError)? {
            if let Some(name) = entry.file_name().to_str() {
                final_contents.push(name.to_string());
            }
        }
        info!("[CLONE] Final workspace contents: {:?}", final_contents);
        info!("[CLONE] ===== Clone completed successfully for job_id: {} =====", job_id);

        Ok(workspace)
    }

    async fn perform_clone(
        &self,
        workspace: &Path,
        repo: &RepositoryInfo,
        auth: &ServiceAuth,
    ) -> Result<(), ExecutionError> {
        info!("[CLONE] Starting clone process for workspace: {}", workspace.display());
        
        // Clone to a temporary directory first, then move contents to workspace
        // This is more reliable than cloning directly into workspace with "."
        let workspace_parent = workspace.parent()
            .ok_or_else(|| ExecutionError::CloneError(GitError::CommandFailed(
                "Workspace has no parent directory".to_string()
            )))?;
        
        info!("[CLONE] Workspace parent: {}", workspace_parent.display());
        
        let workspace_name = workspace.file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| ExecutionError::CloneError(GitError::CommandFailed(
                "Invalid workspace name".to_string()
            )))?;
        
        info!("[CLONE] Workspace name: {}", workspace_name);
        
        let temp_dir = workspace_parent.join(format!("{}.tmp", workspace_name));
        info!("[CLONE] Temporary directory: {}", temp_dir.display());

        // Clean up temp directory if it exists
        if temp_dir.exists() {
            info!("[CLONE] Removing existing temp directory: {}", temp_dir.display());
            fs::remove_dir_all(&temp_dir)
                .await
                .map_err(ExecutionError::IoError)?;
        }

        info!("[CLONE] Building git clone command");
        let mut cmd = Command::new("git");
        cmd.arg("clone");

        if let Some(depth) = self.clone_depth {
            info!("[CLONE] Using shallow clone with depth: {}", depth);
            cmd.arg("--depth").arg(depth.to_string());
        }

        // Clone to temporary directory
        cmd.arg("--single-branch")
            .arg("--branch")
            .arg(&repo.ref_name)
            .arg(&repo.clone_url)
            .arg(&temp_dir)
            .current_dir(workspace_parent);
        
        info!("[CLONE] Git command: git clone --single-branch --branch {} {} {}", 
              repo.ref_name, repo.clone_url, temp_dir.display());
        info!("[CLONE] Working directory: {}", workspace_parent.display());

        // Set up authentication
        info!("[CLONE] Setting up authentication");
        self.setup_auth(&mut cmd, auth)?;

        info!("[CLONE] Executing git clone command for repository: {}", repo.clone_url);

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

        let output = output.map_err(|e| {
            error!("[CLONE] Error executing git command: {}", e);
            ExecutionError::CloneError(GitError::IoError(e))
        })?;

        info!("[CLONE] Git clone command completed with status: {:?}", output.status);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        info!("[CLONE] Git clone stdout: {}", stdout);
        info!("[CLONE] Git clone stderr: {}", stderr);

        if !output.status.success() {
            error!(
                "[CLONE] Git clone failed: status={:?}, stderr={}, stdout={}, workspace={}",
                output.status, stderr, stdout, workspace.display()
            );
            if stderr.contains("Authentication") || stderr.contains("Permission denied") {
                return Err(ExecutionError::CloneError(GitError::AuthFailed));
            }
            return Err(ExecutionError::CloneError(GitError::CommandFailed(
                format!("git clone failed: {} (stdout: {})", stderr, stdout),
            )));
        }

        // Verify that files were actually cloned to temp directory
        info!("[CLONE] Verifying clone succeeded - checking for .git in: {}", temp_dir.display());
        if !temp_dir.exists() {
            error!("[CLONE] Temp directory does not exist: {}", temp_dir.display());
            return Err(ExecutionError::CloneError(GitError::CommandFailed(
                format!("Temp directory {} does not exist after clone", temp_dir.display()),
            )));
        }

        if !temp_dir.join(".git").exists() {
            // List what's actually in the temp directory
            let mut entries = fs::read_dir(&temp_dir)
                .await
                .map_err(ExecutionError::IoError)?;
            let mut file_names = Vec::new();
            while let Some(entry) = entries.next_entry().await.map_err(ExecutionError::IoError)? {
                if let Some(name) = entry.file_name().to_str() {
                    file_names.push(name.to_string());
                }
            }
            error!("[CLONE] .git directory not found in temp dir. Contents: {:?}", file_names);
            return Err(ExecutionError::CloneError(GitError::CommandFailed(
                format!("Git clone completed but .git directory not found in {}. Directory contents: {:?}", 
                        temp_dir.display(), file_names),
            )));
        }
        info!("[CLONE] Clone verification successful - .git directory found");

        // Ensure workspace directory exists
        info!("[CLONE] Preparing workspace directory: {}", workspace.display());
        if workspace.exists() {
            info!("[CLONE] Removing existing workspace directory");
            fs::remove_dir_all(workspace)
                .await
                .map_err(ExecutionError::IoError)?;
        }
        info!("[CLONE] Creating workspace directory");
        fs::create_dir_all(workspace)
            .await
            .map_err(ExecutionError::IoError)?;

        // Move all contents from temp directory to workspace
        info!("[CLONE] Moving contents from temp directory to workspace");
        let mut entries = fs::read_dir(&temp_dir)
            .await
            .map_err(ExecutionError::IoError)?;
        
        let mut moved_count = 0;
        while let Some(entry) = entries.next_entry().await.map_err(ExecutionError::IoError)? {
            let entry_path = entry.path();
            let file_name = entry_path.file_name()
                .ok_or_else(|| ExecutionError::CloneError(GitError::CommandFailed(
                    "Invalid file name".to_string()
                )))?;
            let dest_path = workspace.join(file_name);
            
            info!("[CLONE] Moving {} to {}", entry_path.display(), dest_path.display());
            // Move file/directory
            fs::rename(&entry_path, &dest_path)
                .await
                .map_err(|e| {
                    error!("[CLONE] Failed to move {} to {}: {}", entry_path.display(), dest_path.display(), e);
                    ExecutionError::CloneError(GitError::CommandFailed(
                        format!("Failed to move {} to {}: {}", entry_path.display(), dest_path.display(), e)
                    ))
                })?;
            moved_count += 1;
        }
        info!("[CLONE] Moved {} items from temp directory to workspace", moved_count);

        // Remove temporary directory (should be empty now)
        info!("[CLONE] Removing temporary directory: {}", temp_dir.display());
        fs::remove_dir(&temp_dir)
            .await
            .map_err(ExecutionError::IoError)?;

        // Final verification - list workspace contents
        info!("[CLONE] Verifying final workspace contents");
        let mut final_entries = fs::read_dir(workspace)
            .await
            .map_err(ExecutionError::IoError)?;
        let mut final_files = Vec::new();
        while let Some(entry) = final_entries.next_entry().await.map_err(ExecutionError::IoError)? {
            if let Some(name) = entry.file_name().to_str() {
                final_files.push(name.to_string());
            }
        }
        info!("[CLONE] Final workspace contents: {:?}", final_files);

        info!("[CLONE] Successfully cloned repository into {}", workspace.display());
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
