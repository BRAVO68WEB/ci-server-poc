//! Trait for artifact storage backends

use crate::models::error::ExecutionError;
use crate::models::types::ArtifactInfo;
use std::path::Path;
use uuid::Uuid;

#[async_trait::async_trait]
pub trait ArtifactStorage: Send + Sync {
    /// Initialize the storage backend
    async fn initialize(&self) -> Result<(), ExecutionError>;

    /// Upload artifact data
    async fn upload_artifact(
        &self,
        job_id: Uuid,
        artifact_name: String,
        data: Vec<u8>,
    ) -> Result<ArtifactInfo, ExecutionError>;

    /// Upload artifact from file path
    async fn upload_artifact_from_file(
        &self,
        job_id: Uuid,
        artifact_name: String,
        file_path: &Path,
    ) -> Result<ArtifactInfo, ExecutionError>;

    /// Download artifact
    async fn download_artifact(
        &self,
        job_id: Uuid,
        artifact_name: &str,
    ) -> Result<Vec<u8>, ExecutionError>;

    /// List artifacts for a job
    async fn list_artifacts(&self, job_id: Uuid) -> Vec<ArtifactInfo>;

    /// Cleanup old artifacts
    async fn cleanup_old_artifacts(&self) -> Result<(), ExecutionError>;
}

