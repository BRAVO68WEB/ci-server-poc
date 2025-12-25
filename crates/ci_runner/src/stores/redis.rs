//! Redis-based job persistence

use crate::models::error::ExecutionError;
use crate::models::types::{JobEvent, JobResult, LogEntry};
use chrono::{DateTime, Utc};
use redis::{AsyncCommands, Client};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobState {
    pub job_id: Uuid,
    pub run_id: Uuid,
    pub event: JobEvent,
    pub status: JobStateStatus,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub result: Option<JobResult>,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum JobStateStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl From<crate::stores::memory::JobStateStatus> for JobStateStatus {
    fn from(status: crate::stores::memory::JobStateStatus) -> Self {
        match status {
            crate::stores::memory::JobStateStatus::Pending => JobStateStatus::Pending,
            crate::stores::memory::JobStateStatus::Running => JobStateStatus::Running,
            crate::stores::memory::JobStateStatus::Completed => JobStateStatus::Completed,
            crate::stores::memory::JobStateStatus::Failed => JobStateStatus::Failed,
            crate::stores::memory::JobStateStatus::Cancelled => JobStateStatus::Cancelled,
        }
    }
}

impl From<JobStateStatus> for crate::stores::memory::JobStateStatus {
    fn from(status: JobStateStatus) -> Self {
        match status {
            JobStateStatus::Pending => crate::stores::memory::JobStateStatus::Pending,
            JobStateStatus::Running => crate::stores::memory::JobStateStatus::Running,
            JobStateStatus::Completed => crate::stores::memory::JobStateStatus::Completed,
            JobStateStatus::Failed => crate::stores::memory::JobStateStatus::Failed,
            JobStateStatus::Cancelled => crate::stores::memory::JobStateStatus::Cancelled,
        }
    }
}

pub struct RedisStore {
    client: Arc<Client>,
    prefix: String,
    max_history: usize,
}

impl RedisStore {
    pub async fn new(url: &str, prefix: String, max_history: usize) -> Result<Self, ExecutionError> {
        let client = Client::open(url)
            .map_err(|e| ExecutionError::ConfigError(format!("Failed to connect to Redis: {}", e)))?;
        
        Ok(Self {
            client: Arc::new(client),
            prefix,
            max_history,
        })
    }

    fn job_key(&self, job_id: Uuid) -> String {
        format!("{}:job:{}", self.prefix, job_id)
    }

    fn logs_key(&self, job_id: Uuid) -> String {
        format!("{}:logs:{}", self.prefix, job_id)
    }

    fn index_key(&self) -> String {
        format!("{}:index", self.prefix)
    }

    pub async fn create_job(&self, event: JobEvent) -> Result<(), ExecutionError> {
        let mut conn = self.client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| ExecutionError::ConfigError(format!("Redis connection error: {}", e)))?;

        let now = Utc::now();
        let state = JobState {
            job_id: event.job_id,
            run_id: event.run_id,
            event: event.clone(),
            status: JobStateStatus::Pending,
            started_at: now,
            finished_at: None,
            result: None,
            error: None,
            created_at: now,
            updated_at: now,
        };

        let key = self.job_key(event.job_id);
        let data = serde_json::to_string(&state)
            .map_err(|e| ExecutionError::ConfigError(format!("Serialization error: {}", e)))?;

        conn.set::<_, _, ()>(&key, &data).await
            .map_err(|e| ExecutionError::ConfigError(format!("Redis SET error: {}", e)))?;

        // Add to index
        conn.zadd::<_, _, _, ()>(&self.index_key(), &key, now.timestamp()).await
            .map_err(|e| ExecutionError::ConfigError(format!("Redis ZADD error: {}", e)))?;

        // Enforce max history (spawn separate task to avoid blocking)
        let store_clone = Self {
            client: Arc::clone(&self.client),
            prefix: self.prefix.clone(),
            max_history: self.max_history,
        };
        tokio::spawn(async move {
            if let Err(e) = store_clone.enforce_max_history().await {
                tracing::warn!("Failed to enforce max history: {}", e);
            }
        });

        Ok(())
    }

    pub async fn update_job_status(&self, job_id: Uuid, status: JobStateStatus) -> Result<(), ExecutionError> {
        let mut conn = self.client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| ExecutionError::ConfigError(format!("Redis connection error: {}", e)))?;

        let key = self.job_key(job_id);
        let data: String = conn.get(&key).await
            .map_err(|e| ExecutionError::ConfigError(format!("Redis GET error: {}", e)))?;

        let mut state: JobState = serde_json::from_str(&data)
            .map_err(|e| ExecutionError::ConfigError(format!("Deserialization error: {}", e)))?;

        state.status = status;
        state.updated_at = Utc::now();
        if matches!(status, JobStateStatus::Completed | JobStateStatus::Failed | JobStateStatus::Cancelled) {
            state.finished_at = Some(Utc::now());
        }

        let updated_data = serde_json::to_string(&state)
            .map_err(|e| ExecutionError::ConfigError(format!("Serialization error: {}", e)))?;

        conn.set::<_, _, ()>(&key, &updated_data).await
            .map_err(|e| ExecutionError::ConfigError(format!("Redis SET error: {}", e)))?;

        Ok(())
    }

    pub async fn set_job_result(&self, job_id: Uuid, result: JobResult) -> Result<(), ExecutionError> {
        let mut conn = self.client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| ExecutionError::ConfigError(format!("Redis connection error: {}", e)))?;

        let key = self.job_key(job_id);
        let data: String = conn.get(&key).await
            .map_err(|e| ExecutionError::ConfigError(format!("Redis GET error: {}", e)))?;

        let mut state: JobState = serde_json::from_str(&data)
            .map_err(|e| ExecutionError::ConfigError(format!("Deserialization error: {}", e)))?;

        state.result = Some(result.clone());
        state.status = JobStateStatus::from(crate::stores::memory::JobStateStatus::from(result.status));
        state.finished_at = Some(Utc::now());
        state.updated_at = Utc::now();

        let updated_data = serde_json::to_string(&state)
            .map_err(|e| ExecutionError::ConfigError(format!("Serialization error: {}", e)))?;

        conn.set::<_, _, ()>(&key, &updated_data).await
            .map_err(|e| ExecutionError::ConfigError(format!("Redis SET error: {}", e)))?;

        Ok(())
    }

    pub async fn set_job_error(&self, job_id: Uuid, error: String) -> Result<(), ExecutionError> {
        let mut conn = self.client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| ExecutionError::ConfigError(format!("Redis connection error: {}", e)))?;

        let key = self.job_key(job_id);
        let data: String = conn.get(&key).await
            .map_err(|e| ExecutionError::ConfigError(format!("Redis GET error: {}", e)))?;

        let mut state: JobState = serde_json::from_str(&data)
            .map_err(|e| ExecutionError::ConfigError(format!("Deserialization error: {}", e)))?;

        state.error = Some(error.clone());
        state.status = JobStateStatus::Failed;
        state.finished_at = Some(Utc::now());
        state.updated_at = Utc::now();

        let updated_data = serde_json::to_string(&state)
            .map_err(|e| ExecutionError::ConfigError(format!("Serialization error: {}", e)))?;

        conn.set::<_, _, ()>(&key, &updated_data).await
            .map_err(|e| ExecutionError::ConfigError(format!("Redis SET error: {}", e)))?;

        Ok(())
    }

    pub async fn get_job(&self, job_id: Uuid) -> Result<Option<JobState>, ExecutionError> {
        let mut conn = self.client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| ExecutionError::ConfigError(format!("Redis connection error: {}", e)))?;

        let key = self.job_key(job_id);
        let data: Option<String> = conn.get(&key).await
            .map_err(|e| ExecutionError::ConfigError(format!("Redis GET error: {}", e)))?;

        match data {
            Some(d) => {
                let state: JobState = serde_json::from_str(&d)
                    .map_err(|e| ExecutionError::ConfigError(format!("Deserialization error: {}", e)))?;
                Ok(Some(state))
            }
            None => Ok(None),
        }
    }

    pub async fn list_jobs(
        &self,
        limit: usize,
        offset: usize,
        status_filter: Option<JobStateStatus>,
    ) -> Result<Vec<JobState>, ExecutionError> {
        let mut conn = self.client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| ExecutionError::ConfigError(format!("Redis connection error: {}", e)))?;

        let index_key = self.index_key();
        let total: usize = conn.zcard(&index_key).await
            .map_err(|e| ExecutionError::ConfigError(format!("Redis ZCARD error: {}", e)))?;

        let start = total.saturating_sub(offset + limit);
        let end = total.saturating_sub(offset).saturating_sub(1);
        
        let keys: Vec<String> = conn.zrevrange(&index_key, start as isize, end as isize).await
            .map_err(|e| ExecutionError::ConfigError(format!("Redis ZREVRANGE error: {}", e)))?;

        let mut jobs = Vec::new();
        for key in keys {
            let data: Option<String> = conn.get(&key).await
                .map_err(|e| ExecutionError::ConfigError(format!("Redis GET error: {}", e)))?;
            
            if let Some(d) = data {
                if let Ok(state) = serde_json::from_str::<JobState>(&d) {
                    if let Some(filter) = status_filter {
                        if state.status == filter {
                            jobs.push(state);
                        }
                    } else {
                        jobs.push(state);
                    }
                }
            }
        }

        Ok(jobs)
    }

    pub async fn add_log(&self, job_id: Uuid, log_entry: LogEntry) -> Result<(), ExecutionError> {
        let mut conn = self.client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| ExecutionError::ConfigError(format!("Redis connection error: {}", e)))?;

        let key = self.logs_key(job_id);
        let data = serde_json::to_string(&log_entry)
            .map_err(|e| ExecutionError::ConfigError(format!("Serialization error: {}", e)))?;

        conn.lpush::<_, _, ()>(&key, &data).await
            .map_err(|e| ExecutionError::ConfigError(format!("Redis LPUSH error: {}", e)))?;

        // Trim logs to prevent unbounded growth
        conn.ltrim::<_, ()>(&key, 0, 10000).await
            .map_err(|e| ExecutionError::ConfigError(format!("Redis LTRIM error: {}", e)))?;

        Ok(())
    }

    pub async fn get_logs(&self, job_id: Uuid, limit: Option<usize>) -> Result<Vec<LogEntry>, ExecutionError> {
        let mut conn = self.client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| ExecutionError::ConfigError(format!("Redis connection error: {}", e)))?;

        let key = self.logs_key(job_id);
        let count = limit.unwrap_or(1000);
        
        let data: Vec<String> = conn.lrange(&key, 0, count as isize).await
            .map_err(|e| ExecutionError::ConfigError(format!("Redis LRANGE error: {}", e)))?;

        let mut logs = Vec::new();
        for item in data {
            if let Ok(log) = serde_json::from_str::<LogEntry>(&item) {
                logs.push(log);
            }
        }

        Ok(logs)
    }

    pub async fn count_jobs(&self, status_filter: Option<JobStateStatus>) -> Result<usize, ExecutionError> {
        let mut conn = self.client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| ExecutionError::ConfigError(format!("Redis connection error: {}", e)))?;

        if status_filter.is_none() {
            let count: usize = conn.zcard(self.index_key()).await
                .map_err(|e| ExecutionError::ConfigError(format!("Redis ZCARD error: {}", e)))?;
            return Ok(count);
        }

        // If filtering by status, we need to scan all jobs
        let all_jobs = self.list_jobs(10000, 0, None).await?;
        Ok(all_jobs.iter().filter(|j| {
            status_filter.map(|f| j.status == f).unwrap_or(true)
        }).count())
    }

    async fn enforce_max_history(&self) -> Result<(), ExecutionError> {
        let mut conn = self.client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| ExecutionError::ConfigError(format!("Redis connection error: {}", e)))?;
            
        let count: usize = conn.zcard(self.index_key()).await
            .map_err(|e| ExecutionError::ConfigError(format!("Redis ZCARD error: {}", e)))?;

        if count > self.max_history {
            let to_remove = count - self.max_history;
            let keys: Vec<String> = conn.zrange(self.index_key(), 0, to_remove as isize).await
                .map_err(|e| ExecutionError::ConfigError(format!("Redis ZRANGE error: {}", e)))?;

            for key in keys {
                // Extract job_id from key
                if let Some(job_id_str) = key.strip_prefix(&format!("{}:job:", self.prefix)) {
                    if let Ok(job_id) = Uuid::parse_str(job_id_str) {
                        // Delete job and logs
                        let _: () = conn.del(&key).await
                            .map_err(|e| ExecutionError::ConfigError(format!("Redis DEL error: {}", e)))?;
                        let _: () = conn.del(self.logs_key(job_id)).await
                            .map_err(|e| ExecutionError::ConfigError(format!("Redis DEL error: {}", e)))?;
                    }
                }
                // Remove from index
                let _: () = conn.zrem(self.index_key(), &key).await
                    .map_err(|e| ExecutionError::ConfigError(format!("Redis ZREM error: {}", e)))?;
            }
        }

        Ok(())
    }

    pub async fn search_jobs(
        &self,
        query: &JobSearchQuery,
    ) -> Result<Vec<JobState>, ExecutionError> {
        // Simple search implementation - can be enhanced with Redis Search
        let all_jobs = self.list_jobs(10000, 0, query.status).await?;
        
        let mut results = all_jobs.into_iter().filter(|job| {
            if let Some(ref repo_owner) = query.repository_owner {
                if job.event.repository.owner != *repo_owner {
                    return false;
                }
            }
            if let Some(ref repo_name) = query.repository_name {
                if job.event.repository.name != *repo_name {
                    return false;
                }
            }
            if let Some(ref commit_sha) = query.commit_sha {
                if job.event.repository.commit_sha != *commit_sha {
                    return false;
                }
            }
            if let Some(ref ref_name) = query.ref_name {
                if job.event.repository.ref_name != *ref_name {
                    return false;
                }
            }
            true
        }).collect::<Vec<_>>();

        results.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(results.into_iter().skip(query.offset.unwrap_or(0)).take(query.limit.unwrap_or(50)).collect())
    }
}

#[derive(Debug, Clone)]
pub struct JobSearchQuery {
    pub status: Option<JobStateStatus>,
    pub repository_owner: Option<String>,
    pub repository_name: Option<String>,
    pub commit_sha: Option<String>,
    pub ref_name: Option<String>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

