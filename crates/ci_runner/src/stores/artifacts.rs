//! Artifact storage and management

use crate::models::error::ExecutionError;
use crate::models::types::ArtifactInfo;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::RwLock;
use uuid::Uuid;
use tracing::info;

pub struct ArtifactStore {
    base_path: PathBuf,
    artifacts: Arc<RwLock<HashMap<Uuid, Vec<ArtifactInfo>>>>,
}

impl ArtifactStore {
    pub fn new(base_path: PathBuf) -> Self {
        Self {
            base_path,
            artifacts: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn initialize(&self) -> Result<(), ExecutionError> {
        fs::create_dir_all(&self.base_path).await
            .map_err(ExecutionError::IoError)?;
        Ok(())
    }

    pub async fn upload_artifact(
        &self,
        job_id: Uuid,
        artifact_name: String,
        data: Vec<u8>,
    ) -> Result<ArtifactInfo, ExecutionError> {
        let job_dir = self.base_path.join(job_id.to_string());
        fs::create_dir_all(&job_dir).await
            .map_err(ExecutionError::IoError)?;

        let artifact_path = job_dir.join(&artifact_name);
        let mut file = fs::File::create(&artifact_path).await
            .map_err(ExecutionError::IoError)?;
        
        file.write_all(&data).await
            .map_err(ExecutionError::IoError)?;
        
        let metadata = fs::metadata(&artifact_path).await
            .map_err(ExecutionError::IoError)?;
        let size = metadata.len();

        // Calculate checksum (simple hash for now)
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        data.hash(&mut hasher);
        let checksum = format!("{:x}", hasher.finish());

        let artifact_info = ArtifactInfo {
            name: artifact_name.clone(),
            path: artifact_path.to_string_lossy().to_string(),
            size,
            checksum,
        };

        // Track artifact
        let mut artifacts = self.artifacts.write().await;
        artifacts.entry(job_id).or_insert_with(Vec::new).push(artifact_info.clone());

        info!(job_id = %job_id, artifact = %artifact_name, size = size, "Artifact uploaded");
        Ok(artifact_info)
    }

    pub async fn download_artifact(
        &self,
        job_id: Uuid,
        artifact_name: &str,
    ) -> Result<Vec<u8>, ExecutionError> {
        let job_dir = self.base_path.join(job_id.to_string());
        let artifact_path = job_dir.join(artifact_name);

        if !artifact_path.exists() {
            return Err(ExecutionError::IoError(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Artifact {} not found for job {}", artifact_name, job_id),
            )));
        }

        let mut file = fs::File::open(&artifact_path).await
            .map_err(ExecutionError::IoError)?;
        
        let mut data = Vec::new();
        file.read_to_end(&mut data).await
            .map_err(ExecutionError::IoError)?;

        Ok(data)
    }

    pub async fn list_artifacts(&self, job_id: Uuid) -> Vec<ArtifactInfo> {
        let artifacts = self.artifacts.read().await;
        artifacts.get(&job_id).cloned().unwrap_or_default()
    }

    pub async fn cleanup_old_artifacts(&self) -> Result<(), ExecutionError> {
        // This is a simplified cleanup - in production, you'd want to track creation times
        // For now, we'll rely on the job store's cleanup
        Ok(())
    }
}

