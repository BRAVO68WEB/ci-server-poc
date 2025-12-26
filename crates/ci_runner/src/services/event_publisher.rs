use crate::config::GitServerConfig;
use crate::models::error::PublishError;
use crate::models::types::{JobCompletionEvent, JobStatus, StepSummary};
use reqwest::Client;
use std::collections::HashMap;
use std::time::Duration;
use tracing::{info, warn};

const MAX_RETRIES: u32 = 3;
const INITIAL_BACKOFF_MS: u64 = 500;

pub struct EventPublisher {
    http_client: Client,
    config: GitServerConfig,
    auth_token: String,
}

impl EventPublisher {
    pub fn new(config: GitServerConfig, auth_token: String) -> Self {
        let http_client = Client::builder()
            .timeout(config.api_timeout)
            .build()
            .expect("Failed to create HTTP client");

        Self {
            http_client,
            config,
            auth_token,
        }
    }

    pub async fn publish(&self, event: JobCompletionEvent) -> Result<(), PublishError> {
        info!(
            job_id = %event.job_id,
            status = ?event.status,
            "Publishing job completion event"
        );

        // Publish to HTTP API with retry logic
        self.publish_to_http_with_retry(&event).await?;

        Ok(())
    }

    async fn publish_to_http_with_retry(&self, event: &JobCompletionEvent) -> Result<(), PublishError> {
        let mut last_error = None;
        
        for attempt in 0..MAX_RETRIES {
            match self.publish_to_http(event).await {
                Ok(()) => return Ok(()),
                Err(e) => {
                    let should_retry = matches!(&e, 
                        PublishError::HttpError(status) if *status == 429 || *status >= 500
                    ) || matches!(&e, PublishError::NetworkError(_));
                    
                    if should_retry && attempt < MAX_RETRIES - 1 {
                        let backoff = Duration::from_millis(INITIAL_BACKOFF_MS * 2u64.pow(attempt));
                        warn!(
                            job_id = %event.job_id,
                            attempt = attempt + 1,
                            backoff_ms = backoff.as_millis(),
                            error = %e,
                            "Retrying event publish after backoff"
                        );
                        tokio::time::sleep(backoff).await;
                        last_error = Some(e);
                    } else {
                        return Err(e);
                    }
                }
            }
        }
        
        Err(last_error.unwrap_or(PublishError::HttpError(500)))
    }

    async fn publish_to_http(&self, event: &JobCompletionEvent) -> Result<(), PublishError> {
        let url = format!(
            "{}/api/v1/ci/jobs/{}/complete",
            self.config.base_url, event.job_id
        );

        let response = self
            .http_client
            .post(&url)
            .bearer_auth(&self.auth_token)
            .json(event)
            .send()
            .await
            .map_err(PublishError::NetworkError)?;

        if !response.status().is_success() {
            warn!(
                "Failed to publish event for job {}: HTTP {}",
                event.job_id,
                response.status().as_u16()
            );
            return Err(PublishError::HttpError(response.status().as_u16()));
        }

        info!(
            job_id = %event.job_id,
            "Event published successfully"
        );

        Ok(())
    }
}

impl crate::models::types::JobCompletionEvent {
    pub async fn from_result(
        job_id: uuid::Uuid,
        run_id: uuid::Uuid,
        result: &crate::models::types::JobResult,
        workspace_path: Option<&std::path::Path>,
    ) -> Self {
        let steps: Vec<StepSummary> = result
            .steps
            .iter()
            .map(|step| StepSummary {
                name: step.name.clone(),
                status: if step.exit_code == 0 {
                    JobStatus::Success
                } else {
                    JobStatus::Failed
                },
                exit_code: step.exit_code,
                duration: step
                    .finished_at
                    .signed_duration_since(step.started_at)
                    .to_std()
                    .unwrap_or_default(),
            })
            .collect();

        // Collect artifacts from workspace if available
        // Note: Artifacts from post steps are collected and uploaded separately in main.rs
        let artifacts = if let Some(workspace) = workspace_path {
            let collector = crate::services::artifact_collector::ArtifactCollector::new();
            collector.collect_artifacts(workspace, None).await.unwrap_or_default()
        } else {
            Vec::new()
        };

        Self {
            job_id,
            run_id,
            status: result.status,
            started_at: result.started_at,
            finished_at: result.finished_at,
            duration: result.duration(),
            exit_code: result.steps.last().map(|s| s.exit_code).unwrap_or(0),
            steps,
            artifacts,
            metadata: HashMap::new(),
        }
    }
}
