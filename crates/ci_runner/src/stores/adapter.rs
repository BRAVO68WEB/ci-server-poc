//! Store adapter for supporting both in-memory and Redis backends

use crate::models::error::ExecutionError;
use crate::models::types::{JobEvent, JobResult, LogEntry};
use async_trait::async_trait;
use uuid::Uuid;

#[async_trait]
pub trait StoreAdapter: Send + Sync {
    async fn create_job(&self, event: JobEvent) -> Result<(), ExecutionError>;
    async fn update_job_status(&self, job_id: Uuid, status: crate::stores::memory::JobStateStatus) -> Result<(), ExecutionError>;
    async fn set_job_result(&self, job_id: Uuid, result: JobResult) -> Result<(), ExecutionError>;
    async fn set_job_error(&self, job_id: Uuid, error: String) -> Result<(), ExecutionError>;
    async fn get_job(&self, job_id: Uuid) -> Result<Option<crate::stores::memory::JobState>, ExecutionError>;
    async fn list_jobs(&self, limit: usize, offset: usize, status_filter: Option<crate::stores::memory::JobStateStatus>) -> Result<Vec<crate::stores::memory::JobState>, ExecutionError>;
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

    async fn get_job(&self, job_id: Uuid) -> Result<Option<crate::stores::memory::JobState>, ExecutionError> {
        Ok(self.store.get_job(job_id).await)
    }

    async fn list_jobs(&self, limit: usize, offset: usize, status_filter: Option<crate::stores::memory::JobStateStatus>) -> Result<Vec<crate::stores::memory::JobState>, ExecutionError> {
        Ok(self.store.list_jobs(limit, offset, status_filter).await)
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

