//! Store adapter for supporting both in-memory and Redis backends

use crate::models::error::ExecutionError;
use crate::models::types::{ArtifactInfo, JobEvent, JobResult, LogEntry};
use async_trait::async_trait;
use uuid::Uuid;

/// Query parameters for searching jobs
#[derive(Debug, Clone, Default)]
pub struct JobSearchQuery {
    pub status: Option<crate::stores::memory::JobStateStatus>,
    pub repository_owner: Option<String>,
    pub repository_name: Option<String>,
    pub commit_sha: Option<String>,
    pub ref_name: Option<String>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

#[async_trait]
pub trait StoreAdapter: Send + Sync {
    async fn create_job(&self, event: JobEvent) -> Result<(), ExecutionError>;
    async fn update_job_status(&self, job_id: Uuid, status: crate::stores::memory::JobStateStatus) -> Result<(), ExecutionError>;
    async fn set_job_result(&self, job_id: Uuid, result: JobResult) -> Result<(), ExecutionError>;
    async fn set_job_error(&self, job_id: Uuid, error: String) -> Result<(), ExecutionError>;
    async fn set_job_artifacts(&self, job_id: Uuid, artifacts: Vec<ArtifactInfo>) -> Result<(), ExecutionError>;
    async fn get_job(&self, job_id: Uuid) -> Result<Option<crate::stores::memory::JobState>, ExecutionError>;
    async fn list_jobs(&self, limit: usize, offset: usize, status_filter: Option<crate::stores::memory::JobStateStatus>) -> Result<Vec<crate::stores::memory::JobState>, ExecutionError>;
    async fn search_jobs(&self, query: &JobSearchQuery) -> Result<Vec<crate::stores::memory::JobState>, ExecutionError>;
    async fn add_log(&self, job_id: Uuid, log_entry: LogEntry) -> Result<(), ExecutionError>;
    async fn get_logs(&self, job_id: Uuid, limit: Option<usize>) -> Result<Vec<LogEntry>, ExecutionError>;
    async fn count_jobs(&self, status_filter: Option<crate::stores::memory::JobStateStatus>) -> Result<usize, ExecutionError>;
}

// In-memory store adapter
pub struct InMemoryStoreAdapter {
    store: std::sync::Arc<crate::stores::memory::JobStore>,
}

impl InMemoryStoreAdapter {
    pub fn new(store: std::sync::Arc<crate::stores::memory::JobStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl StoreAdapter for InMemoryStoreAdapter {
    async fn create_job(&self, event: JobEvent) -> Result<(), ExecutionError> {
        self.store.create_job(event).await.map_err(ExecutionError::ConfigError)
    }

    async fn update_job_status(&self, job_id: Uuid, status: crate::stores::memory::JobStateStatus) -> Result<(), ExecutionError> {
        self.store.update_job_status(job_id, status).await.map_err(ExecutionError::ConfigError)
    }

    async fn set_job_result(&self, job_id: Uuid, result: JobResult) -> Result<(), ExecutionError> {
        self.store.set_job_result(job_id, result).await.map_err(ExecutionError::ConfigError)
    }

    async fn set_job_error(&self, job_id: Uuid, error: String) -> Result<(), ExecutionError> {
        self.store.set_job_error(job_id, error).await.map_err(ExecutionError::ConfigError)
    }

    async fn set_job_artifacts(&self, job_id: Uuid, artifacts: Vec<ArtifactInfo>) -> Result<(), ExecutionError> {
        self.store.set_job_artifacts(job_id, artifacts).await.map_err(ExecutionError::ConfigError)
    }

    async fn get_job(&self, job_id: Uuid) -> Result<Option<crate::stores::memory::JobState>, ExecutionError> {
        Ok(self.store.get_job(job_id).await)
    }

    async fn list_jobs(&self, limit: usize, offset: usize, status_filter: Option<crate::stores::memory::JobStateStatus>) -> Result<Vec<crate::stores::memory::JobState>, ExecutionError> {
        Ok(self.store.list_jobs(limit, offset, status_filter).await)
    }

    async fn search_jobs(&self, query: &JobSearchQuery) -> Result<Vec<crate::stores::memory::JobState>, ExecutionError> {
        Ok(self.store.search_jobs(query).await)
    }

    async fn add_log(&self, job_id: Uuid, log_entry: LogEntry) -> Result<(), ExecutionError> {
        self.store.add_log(job_id, log_entry).await;
        Ok(())
    }

    async fn get_logs(&self, job_id: Uuid, limit: Option<usize>) -> Result<Vec<LogEntry>, ExecutionError> {
        Ok(self.store.get_logs(job_id, limit).await)
    }

    async fn count_jobs(&self, status_filter: Option<crate::stores::memory::JobStateStatus>) -> Result<usize, ExecutionError> {
        Ok(self.store.count_jobs(status_filter).await)
    }
}

// Redis store adapter
pub struct RedisStoreAdapter {
    store: std::sync::Arc<crate::stores::redis::RedisStore>,
}

impl RedisStoreAdapter {
    pub fn new(store: std::sync::Arc<crate::stores::redis::RedisStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl StoreAdapter for RedisStoreAdapter {
    async fn create_job(&self, event: JobEvent) -> Result<(), ExecutionError> {
        self.store.create_job(event).await
    }

    async fn update_job_status(&self, job_id: Uuid, status: crate::stores::memory::JobStateStatus) -> Result<(), ExecutionError> {
        self.store.update_job_status(job_id, status.into()).await
    }

    async fn set_job_result(&self, job_id: Uuid, result: JobResult) -> Result<(), ExecutionError> {
        self.store.set_job_result(job_id, result).await
    }

    async fn set_job_error(&self, job_id: Uuid, error: String) -> Result<(), ExecutionError> {
        self.store.set_job_error(job_id, error).await
    }

    async fn set_job_artifacts(&self, job_id: Uuid, artifacts: Vec<ArtifactInfo>) -> Result<(), ExecutionError> {
        self.store.set_job_artifacts(job_id, artifacts).await
    }

    async fn get_job(&self, job_id: Uuid) -> Result<Option<crate::stores::memory::JobState>, ExecutionError> {
        let redis_state = self.store.get_job(job_id).await?;
        Ok(redis_state.map(|s| s.into()))
    }

    async fn list_jobs(&self, limit: usize, offset: usize, status_filter: Option<crate::stores::memory::JobStateStatus>) -> Result<Vec<crate::stores::memory::JobState>, ExecutionError> {
        let redis_jobs = self.store.list_jobs(limit, offset, status_filter.map(|s| s.into())).await?;
        Ok(redis_jobs.into_iter().map(|j| j.into()).collect())
    }

    async fn search_jobs(&self, query: &JobSearchQuery) -> Result<Vec<crate::stores::memory::JobState>, ExecutionError> {
        let redis_query = crate::stores::redis::JobSearchQuery {
            status: query.status.map(|s| s.into()),
            repository_owner: query.repository_owner.clone(),
            repository_name: query.repository_name.clone(),
            commit_sha: query.commit_sha.clone(),
            ref_name: query.ref_name.clone(),
            limit: query.limit,
            offset: query.offset,
        };
        let redis_jobs = self.store.search_jobs(&redis_query).await?;
        Ok(redis_jobs.into_iter().map(|j| j.into()).collect())
    }

    async fn add_log(&self, job_id: Uuid, log_entry: LogEntry) -> Result<(), ExecutionError> {
        self.store.add_log(job_id, log_entry).await
    }

    async fn get_logs(&self, job_id: Uuid, limit: Option<usize>) -> Result<Vec<LogEntry>, ExecutionError> {
        self.store.get_logs(job_id, limit).await
    }

    async fn count_jobs(&self, status_filter: Option<crate::stores::memory::JobStateStatus>) -> Result<usize, ExecutionError> {
        self.store.count_jobs(status_filter.map(|s| s.into())).await
    }
}

