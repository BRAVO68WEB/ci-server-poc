use crate::error::ExecutionError;
use chrono::{DateTime, Utc};
use std::path::{Path, PathBuf};
use tokio::fs;
use tracing::{error, info};
use uuid::Uuid;

pub struct WorkspaceManager {
    root: PathBuf,
}

impl WorkspaceManager {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
        }
    }

    pub async fn create_workspace(&self, job_id: Uuid) -> Result<PathBuf, ExecutionError> {
        let path = self.root.join(job_id.to_string());
        fs::create_dir_all(&path)
            .await
            .map_err(ExecutionError::IoError)?;

        // Set permissions
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o700);
            fs::set_permissions(&path, perms)
                .await
                .map_err(ExecutionError::IoError)?;
        }

        info!("Created workspace at {}", path.display());
        Ok(path)
    }

    pub async fn cleanup_workspace(&self, job_id: Uuid) -> Result<(), ExecutionError> {
        let path = self.root.join(job_id.to_string());

        if path.exists() {
            info!("Cleaning up workspace: {}", path.display());
            fs::remove_dir_all(&path)
                .await
                .map_err(ExecutionError::IoError)?;
        }

        Ok(())
    }

    pub async fn cleanup_old_workspaces(
        &self,
        max_age: std::time::Duration,
    ) -> Result<(), ExecutionError> {
        let mut entries = fs::read_dir(&self.root)
            .await
            .map_err(ExecutionError::IoError)?;

        let cutoff = Utc::now()
            - chrono::Duration::from_std(max_age).map_err(|e| {
                ExecutionError::IoError(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("Invalid duration: {}", e),
                ))
            })?;

        let mut cleaned = 0;
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(ExecutionError::IoError)?
        {
            let metadata = entry.metadata().await.map_err(ExecutionError::IoError)?;

            if let Ok(modified) = metadata.modified() {
                let modified_dt: DateTime<Utc> = modified.into();
                if modified_dt < cutoff {
                    if let Err(e) = fs::remove_dir_all(entry.path()).await {
                        error!("Failed to remove old workspace {:?}: {}", entry.path(), e);
                    } else {
                        cleaned += 1;
                    }
                }
            }
        }

        if cleaned > 0 {
            info!("Cleaned up {} old workspaces", cleaned);
        }

        Ok(())
    }

    pub async fn get_workspace_size(&self, job_id: Uuid) -> Result<u64, ExecutionError> {
        let path = self.root.join(job_id.to_string());
        self.calculate_size(&path).await
    }

    async fn calculate_size(&self, path: &Path) -> Result<u64, ExecutionError> {
        let mut total = 0u64;

        if !path.exists() {
            return Ok(0);
        }

        let mut stack = vec![path.to_path_buf()];

        while let Some(current) = stack.pop() {
            let metadata = fs::metadata(&current)
                .await
                .map_err(ExecutionError::IoError)?;

            if metadata.is_file() {
                total += metadata.len();
            } else if metadata.is_dir() {
                let mut entries = fs::read_dir(&current)
                    .await
                    .map_err(ExecutionError::IoError)?;

                while let Some(entry) = entries
                    .next_entry()
                    .await
                    .map_err(ExecutionError::IoError)?
                {
                    stack.push(entry.path());
                }
            }
        }

        Ok(total)
    }
}
