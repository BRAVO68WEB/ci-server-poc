//! Artifact discovery and collection from job workspaces

use crate::models::error::ExecutionError;
use crate::models::types::ArtifactInfo;
use std::path::Path;
use tokio::fs;
use tokio::process::Command;
use tracing::info;
use glob::Pattern;

pub struct ArtifactCollector {
    default_patterns: Vec<String>,
}

impl ArtifactCollector {
    pub fn new() -> Self {
        Self {
            default_patterns: vec![
                "**/dist/**".to_string(),
                "**/build/**".to_string(),
                "**/target/**/*.{jar,war,ear}".to_string(),
                "**/*.tar.gz".to_string(),
                "**/*.zip".to_string(),
                "**/*.deb".to_string(),
                "**/*.rpm".to_string(),
            ],
        }
    }

    pub async fn collect_artifacts(
        &self,
        workspace_path: &Path,
        patterns: Option<Vec<String>>,
    ) -> Result<Vec<ArtifactInfo>, ExecutionError> {
        let patterns = patterns.unwrap_or_else(|| self.default_patterns.clone());
        let mut artifacts = Vec::new();

        for pattern_str in patterns {
            let pattern = Pattern::new(&pattern_str)
                .map_err(|e| ExecutionError::ConfigError(format!("Invalid artifact pattern {}: {}", pattern_str, e)))?;

            let base = workspace_path.to_path_buf();
            self.collect_matching_files(&base, &base, &pattern, &mut artifacts).await?;
        }

        info!(workspace = %workspace_path.display(), count = artifacts.len(), "Collected artifacts");
        Ok(artifacts)
    }

    async fn collect_matching_files(
        &self,
        base: &Path,
        current: &Path,
        pattern: &Pattern,
        artifacts: &mut Vec<ArtifactInfo>,
    ) -> Result<(), ExecutionError> {
        if !current.exists() {
            return Ok(());
        }

        let metadata = fs::metadata(current).await
            .map_err(ExecutionError::IoError)?;

        if metadata.is_file() {
            let relative = current.strip_prefix(base)
                .map_err(|e| ExecutionError::IoError(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("Path error: {}", e),
                )))?;

            let relative_str = relative.to_string_lossy().replace('\\', "/");
            if pattern.matches(&relative_str) {
                let artifact = self.create_artifact_info(current, &relative_str).await?;
                artifacts.push(artifact);
            }
        } else if metadata.is_dir() {
            let mut entries = fs::read_dir(current).await
                .map_err(ExecutionError::IoError)?;

            while let Some(entry) = entries.next_entry().await
                .map_err(ExecutionError::IoError)? {
                // Box the recursive call to avoid infinite size
                Box::pin(self.collect_matching_files(base, &entry.path(), pattern, artifacts)).await?;
            }
        }

        Ok(())
    }

    async fn create_artifact_info(&self, path: &Path, relative_path: &str) -> Result<ArtifactInfo, ExecutionError> {
        let path_str = path.to_string_lossy();

        // Get file size using stat
        let size_output = Command::new("wc")
            .args(["--bytes", &*path_str])
            .output()
            .await
            .map_err(ExecutionError::IoError)?;

        // Parse the output from wc --bytes (which is bytes + filename)
        let stdout_str = String::from_utf8_lossy(&size_output.stdout);
        let size: u64 = stdout_str
            .split_whitespace()
            .next()
            .unwrap_or("0")
            .parse()
            .unwrap_or(0);

        // Calculate SHA-256 checksum using sha256sum
        let checksum_output = Command::new("sha256sum")
            .arg(&*path_str)
            .output()
            .await
            .map_err(ExecutionError::IoError)?;

        if !checksum_output.status.success() {
            return Err(ExecutionError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("sha256sum failed: {}", String::from_utf8_lossy(&checksum_output.stderr)),
            )));
        }

        // sha256sum output format: "hash  filename"
        let checksum = String::from_utf8_lossy(&checksum_output.stdout)
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_string();

        let name = path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(relative_path)
            .to_string();

        Ok(ArtifactInfo {
            name,
            path: path.to_string_lossy().to_string(),
            size,
            checksum,
            url: None,
        })
    }
}

impl Default for ArtifactCollector {
    fn default() -> Self {
        Self::new()
    }
}

