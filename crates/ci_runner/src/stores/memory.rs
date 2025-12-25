//! Job state persistence and management

use crate::models::types::{JobEvent, JobResult, JobStatus, LogEntry};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct JobState {
    pub job_id: Uuid,
    pub run_id: Uuid,
    pub event: JobEvent,
    pub status: JobStateStatus,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub result: Option<JobResult>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub enum JobStateStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl From<JobStatus> for JobStateStatus {
    fn from(status: JobStatus) -> Self {
        match status {
            JobStatus::Success => JobStateStatus::Completed,
            JobStatus::Failed => JobStateStatus::Failed,
            JobStatus::Cancelled => JobStateStatus::Cancelled,
            JobStatus::TimedOut => JobStateStatus::Failed,
            JobStatus::SystemError => JobStateStatus::Failed,
        }
    }
}

pub struct JobStore {
    jobs: Arc<RwLock<HashMap<Uuid, JobState>>>,
    logs: Arc<RwLock<HashMap<Uuid, Vec<LogEntry>>>>,
    max_history: usize,
}

impl JobStore {
    pub fn new(max_history: usize) -> Self {
        Self {
            jobs: Arc::new(RwLock::new(HashMap::new())),
            logs: Arc::new(RwLock::new(HashMap::new())),
            max_history,
        }
    }

    pub async fn create_job(&self, event: JobEvent) -> Result<(), String> {
        let mut jobs = self.jobs.write().await;
        
        if jobs.contains_key(&event.job_id) {
            return Err(format!("Job {} already exists", event.job_id));
        }

        let state = JobState {
            job_id: event.job_id,
            run_id: event.run_id,
            event: event.clone(),
            status: JobStateStatus::Pending,
            started_at: Utc::now(),
            finished_at: None,
            result: None,
            error: None,
        };

        jobs.insert(event.job_id, state);
        self.enforce_max_history(&mut jobs).await;
        Ok(())
    }

    pub async fn update_job_status(&self, job_id: Uuid, status: JobStateStatus) -> Result<(), String> {
        let mut jobs = self.jobs.write().await;
        
        if let Some(job) = jobs.get_mut(&job_id) {
            job.status = status;
            if matches!(status, JobStateStatus::Completed | JobStateStatus::Failed | JobStateStatus::Cancelled) {
                job.finished_at = Some(Utc::now());
            }
            Ok(())
        } else {
            Err(format!("Job {} not found", job_id))
        }
    }

    pub async fn set_job_result(&self, job_id: Uuid, result: JobResult) -> Result<(), String> {
        let mut jobs = self.jobs.write().await;
        
        if let Some(job) = jobs.get_mut(&job_id) {
            job.result = Some(result.clone());
            job.status = JobStateStatus::from(result.status);
            job.finished_at = Some(Utc::now());
            Ok(())
        } else {
            Err(format!("Job {} not found", job_id))
        }
    }

    pub async fn set_job_error(&self, job_id: Uuid, error: String) -> Result<(), String> {
        let mut jobs = self.jobs.write().await;
        
        if let Some(job) = jobs.get_mut(&job_id) {
            job.error = Some(error.clone());
            job.status = JobStateStatus::Failed;
            job.finished_at = Some(Utc::now());
            Ok(())
        } else {
            Err(format!("Job {} not found", job_id))
        }
    }

    pub async fn get_job(&self, job_id: Uuid) -> Option<JobState> {
        let jobs = self.jobs.read().await;
        jobs.get(&job_id).cloned()
    }

    pub async fn list_jobs(
        &self,
        limit: usize,
        offset: usize,
        status_filter: Option<JobStateStatus>,
    ) -> Vec<JobState> {
        let jobs = self.jobs.read().await;
        let mut jobs_vec: Vec<JobState> = jobs.values().cloned().collect();
        
        // Filter by status if provided
        if let Some(status) = status_filter {
            jobs_vec.retain(|j| j.status == status);
        }
        
        // Sort by started_at descending (newest first)
        jobs_vec.sort_by(|a, b| b.started_at.cmp(&a.started_at));
        
        // Apply pagination
        jobs_vec.into_iter().skip(offset).take(limit).collect()
    }

    pub async fn add_log(&self, job_id: Uuid, log_entry: LogEntry) {
        let mut logs = self.logs.write().await;
        logs.entry(job_id).or_insert_with(Vec::new).push(log_entry);
    }

    pub async fn get_logs(&self, job_id: Uuid, limit: Option<usize>) -> Vec<LogEntry> {
        let logs = self.logs.read().await;
        if let Some(job_logs) = logs.get(&job_id) {
            let mut result = job_logs.clone();
            if let Some(limit) = limit {
                result.truncate(limit);
            }
            result
        } else {
            Vec::new()
        }
    }

    async fn enforce_max_history(&self, jobs: &mut HashMap<Uuid, JobState>) {
        if jobs.len() > self.max_history {
            let mut jobs_vec: Vec<(Uuid, JobState)> = jobs.drain().collect();
            jobs_vec.sort_by(|a, b| b.1.started_at.cmp(&a.1.started_at));
            
            // Keep only the most recent max_history jobs
            for (id, _) in jobs_vec.iter().skip(self.max_history) {
                // Also remove logs for deleted jobs
                let mut logs = self.logs.write().await;
                logs.remove(id);
            }
            
            // Re-insert the kept jobs
            for (id, state) in jobs_vec.into_iter().take(self.max_history) {
                jobs.insert(id, state);
            }
        }
    }

    pub async fn count_jobs(&self, status_filter: Option<JobStateStatus>) -> usize {
        let jobs = self.jobs.read().await;
        if let Some(status) = status_filter {
            jobs.values().filter(|j| j.status == status).count()
        } else {
            jobs.len()
        }
    }
}

