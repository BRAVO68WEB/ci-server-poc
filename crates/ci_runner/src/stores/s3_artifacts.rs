//! S3-based artifact storage

use crate::config::S3Config;
use crate::models::error::ExecutionError;
use crate::models::types::ArtifactInfo;
use crate::stores::artifact_trait::ArtifactStorage;
use aws_sdk_s3::Client as S3Client;
use aws_sdk_s3::config::{BehaviorVersion, Region};
use aws_sdk_s3::primitives::ByteStream;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tokio::fs;
use tokio::io::AsyncReadExt;
use tokio::sync::RwLock;
use tracing::info;
use uuid::Uuid;

pub struct S3ArtifactStore {
    s3_client: S3Client,
    bucket: String,
    public_url_base: Option<String>,
    artifacts: Arc<RwLock<HashMap<Uuid, Vec<ArtifactInfo>>>>,
}

impl S3ArtifactStore {
    pub async fn new(config: S3Config) -> Result<Self, ExecutionError> {
        // Load credentials from files if paths are provided
        let access_key_id = if let Some(ref path) = config.access_key_id_path {
            Some(tokio::fs::read_to_string(path).await
                .map_err(ExecutionError::IoError)?
                .trim()
                .to_string())
        } else {
            config.access_key_id
        };

        let secret_access_key = if let Some(ref path) = config.secret_access_key_path {
            Some(tokio::fs::read_to_string(path).await
                .map_err(ExecutionError::IoError)?
                .trim()
                .to_string())
        } else {
            config.secret_access_key
        };

        // Build S3 config
        let mut s3_config_builder = aws_sdk_s3::config::Config::builder()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new(config.region.clone()));

        if let Some(endpoint) = &config.endpoint {
            s3_config_builder = s3_config_builder.endpoint_url(endpoint);
        }

        // Set credentials via environment variables if provided
        if let (Some(ak), Some(sk)) = (access_key_id, secret_access_key) {
            std::env::set_var("AWS_ACCESS_KEY_ID", &ak);
            std::env::set_var("AWS_SECRET_ACCESS_KEY", &sk);
        }

        if config.path_style {
            s3_config_builder = s3_config_builder.force_path_style(true);
        }

        let s3_config = s3_config_builder.build();
        let s3_client = S3Client::from_conf(s3_config);

        // Verify bucket exists
        s3_client.head_bucket()
            .bucket(&config.bucket)
            .send()
            .await
            .map_err(|e| ExecutionError::ConfigError(format!("Failed to access S3 bucket {}: {}", config.bucket, e)))?;

        info!(bucket = %config.bucket, "S3 artifact store initialized");

        Ok(Self {
            s3_client,
            bucket: config.bucket,
            public_url_base: config.public_url_base,
            artifacts: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    fn artifact_key(&self, job_id: Uuid, artifact_name: &str) -> String {
        format!("artifacts/{}/{}", job_id, artifact_name)
    }

    fn artifact_url(&self, key: &str) -> Option<String> {
        self.public_url_base.as_ref().map(|base| format!("{}/{}", base.trim_end_matches('/'), key))
    }

    pub async fn upload_artifact(
        &self,
        job_id: Uuid,
        artifact_name: String,
        data: Vec<u8>,
    ) -> Result<ArtifactInfo, ExecutionError> {
        let key = self.artifact_key(job_id, &artifact_name);
        let size = data.len() as u64;

        // Upload to S3
        self.s3_client
            .put_object()
            .bucket(&self.bucket)
            .key(&key)
            .body(ByteStream::from(data.clone()))
            .send()
            .await
            .map_err(|e| ExecutionError::IoError(std::io::Error::other(
                format!("S3 upload failed: {}", e),
            )))?;

        // Calculate checksum
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        data.hash(&mut hasher);
        let checksum = format!("{:x}", hasher.finish());

        let url = self.artifact_url(&key);

        let artifact_info = ArtifactInfo {
            name: artifact_name.clone(),
            path: key.clone(),
            size,
            checksum,
            url: url.clone(),
        };

        // Track artifact
        let mut artifacts = self.artifacts.write().await;
        artifacts.entry(job_id).or_insert_with(Vec::new).push(artifact_info.clone());

        info!(job_id = %job_id, artifact = %artifact_name, size = size, url = ?url, "Artifact uploaded to S3");
        Ok(artifact_info)
    }

    pub async fn upload_artifact_from_file(
        &self,
        job_id: Uuid,
        artifact_name: String,
        file_path: &Path,
    ) -> Result<ArtifactInfo, ExecutionError> {
        let mut file = fs::File::open(file_path).await
            .map_err(ExecutionError::IoError)?;
        let mut data = Vec::new();
        file.read_to_end(&mut data).await
            .map_err(ExecutionError::IoError)?;

        self.upload_artifact(job_id, artifact_name, data).await
    }

    pub async fn download_artifact(
        &self,
        job_id: Uuid,
        artifact_name: &str,
    ) -> Result<Vec<u8>, ExecutionError> {
        let key = self.artifact_key(job_id, artifact_name);

        let response = self.s3_client
            .get_object()
            .bucket(&self.bucket)
            .key(&key)
            .send()
            .await
            .map_err(|e| ExecutionError::IoError(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Artifact {} not found in S3: {}", artifact_name, e),
            )))?;

        let mut data = Vec::new();
        let mut body = response.body;
        while let Some(chunk) = body.next().await {
            let chunk = chunk.map_err(|e| ExecutionError::IoError(std::io::Error::other(
                format!("Failed to read S3 object: {}", e),
            )))?;
            data.extend_from_slice(&chunk);
        }

        Ok(data)
    }

    pub async fn list_artifacts(&self, job_id: Uuid) -> Vec<ArtifactInfo> {
        let artifacts = self.artifacts.read().await;
        artifacts.get(&job_id).cloned().unwrap_or_default()
    }

    pub async fn cleanup_old_artifacts(&self) -> Result<(), ExecutionError> {
        // S3 lifecycle policies should handle cleanup
        // This is a placeholder for manual cleanup if needed
        Ok(())
    }
}

#[async_trait::async_trait]
impl ArtifactStorage for S3ArtifactStore {
    async fn initialize(&self) -> Result<(), ExecutionError> {
        // S3 store is initialized in new()
        Ok(())
    }

    async fn upload_artifact(
        &self,
        job_id: Uuid,
        artifact_name: String,
        data: Vec<u8>,
    ) -> Result<ArtifactInfo, ExecutionError> {
        self.upload_artifact(job_id, artifact_name, data).await
    }

    async fn upload_artifact_from_file(
        &self,
        job_id: Uuid,
        artifact_name: String,
        file_path: &Path,
    ) -> Result<ArtifactInfo, ExecutionError> {
        self.upload_artifact_from_file(job_id, artifact_name, file_path).await
    }

    async fn download_artifact(
        &self,
        job_id: Uuid,
        artifact_name: &str,
    ) -> Result<Vec<u8>, ExecutionError> {
        self.download_artifact(job_id, artifact_name).await
    }

    async fn list_artifacts(&self, job_id: Uuid) -> Vec<ArtifactInfo> {
        self.list_artifacts(job_id).await
    }

    async fn cleanup_old_artifacts(&self) -> Result<(), ExecutionError> {
        self.cleanup_old_artifacts().await
    }
}

