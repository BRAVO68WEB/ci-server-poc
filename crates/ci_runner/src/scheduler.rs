use crate::error::ScheduleError;
use crate::types::JobEvent;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, Semaphore};
use tracing::{error, info, warn};
use uuid::Uuid;

pub struct JobHandle {
    pub job_id: Uuid,
    pub handle:
        tokio::task::JoinHandle<Result<crate::types::JobResult, crate::error::ExecutionError>>,
}

pub struct JobScheduler {
    max_concurrent_jobs: usize,
    current_jobs: Arc<RwLock<HashMap<Uuid, JobHandle>>>,
    semaphore: Arc<Semaphore>,
}

impl JobScheduler {
    pub fn new(max_concurrent_jobs: usize) -> Self {
        Self {
            max_concurrent_jobs,
            current_jobs: Arc::new(RwLock::new(HashMap::new())),
            semaphore: Arc::new(Semaphore::new(max_concurrent_jobs)),
        }
    }

    pub async fn schedule<F, Fut>(&self, job: JobEvent, executor: F) -> Result<(), ScheduleError>
    where
        F: FnOnce(JobEvent) -> Fut + Send + 'static,
        Fut: std::future::Future<
                Output = Result<crate::types::JobResult, crate::error::ExecutionError>,
            > + Send
            + 'static,
    {
        // Check if job already exists
        {
            let jobs = self.current_jobs.read().await;
            if jobs.contains_key(&job.job_id) {
                return Err(ScheduleError::JobExists(job.job_id));
            }
        }

        // Log before acquiring permit
        let available_permits = self.semaphore.available_permits();
        info!(
            job_id = %job.job_id,
            "Scheduling job ({} active jobs)",
            self.max_concurrent_jobs - available_permits
        );

        // Clone semaphore before acquiring permit to avoid borrowing self
        let semaphore = Arc::clone(&self.semaphore);

        // Clone everything we need before spawning - this breaks any reference to self
        let job_id = job.job_id;
        let jobs_for_tracking = Arc::clone(&self.current_jobs);
        let jobs_for_cleanup = Arc::clone(&self.current_jobs);

        // Move executor, job, and semaphore into local variables before spawning
        // This ensures the compiler knows these are owned values, not references
        let executor_owned = executor;
        let job_owned = job;
        let semaphore_owned = semaphore;
        let job_id_owned = job_id;
        let jobs_owned = jobs_for_cleanup;

        // Now spawn with all owned values - no references to self remain
        // Acquire permit inside the spawn to avoid lifetime issues
        let handle = tokio::spawn(async move {
            // Acquire permit (blocks if at capacity)
            let permit = semaphore_owned.acquire().await.map_err(|_| {
                crate::error::ExecutionError::ConfigError(
                    "Failed to acquire semaphore permit".to_string(),
                )
            })?;

            let result = executor_owned(job_owned).await;

            // Release permit
            drop(permit);

            // Remove from tracking
            let _ = jobs_owned.write().await.remove(&job_id_owned);

            // Log completion
            match &result {
                Ok(r) => {
                    info!(
                        job_id = %job_id_owned,
                        status = ?r.status,
                        "Job completed"
                    );
                }
                Err(e) => {
                    error!(
                        job_id = %job_id_owned,
                        error = %e,
                        "Job failed"
                    );
                }
            }

            result
        });

        // Track job (use cloned Arc, not self)
        jobs_for_tracking
            .write()
            .await
            .insert(job_id, JobHandle { job_id, handle });

        Ok(())
    }

    pub async fn cancel_job(&self, job_id: Uuid) -> Result<(), ScheduleError> {
        let mut jobs = self.current_jobs.write().await;

        if let Some(handle) = jobs.remove(&job_id) {
            handle.handle.abort();
            info!(job_id = %job_id, "Job cancelled");
            Ok(())
        } else {
            Err(ScheduleError::JobExists(job_id)) // Job not found
        }
    }

    pub async fn active_jobs(&self) -> usize {
        let jobs = self.current_jobs.read().await;
        jobs.len()
    }

    pub async fn wait_for_completion(&self, max_wait: std::time::Duration) -> Result<(), ()> {
        let start = std::time::Instant::now();

        loop {
            let active = self.active_jobs().await;
            if active == 0 {
                return Ok(());
            }

            if start.elapsed() > max_wait {
                warn!("Graceful shutdown timeout: {} jobs still running", active);
                return Err(());
            }

            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    }
}
