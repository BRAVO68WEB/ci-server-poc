use crate::config::GitServerConfig;
use crate::error::PublishError;
use crate::types::{JobCompletionEvent, JobStatus, StepSummary};
use reqwest::Client;
use std::collections::HashMap;
use tracing::{info, warn};

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

        // Publish to HTTP API
        self.publish_to_http(&event).await?;

        Ok(())
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

        Ok(())
    }
}

impl crate::types::JobCompletionEvent {
    pub fn from_result(
        job_id: uuid::Uuid,
        run_id: uuid::Uuid,
        result: &crate::types::JobResult,
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

        Self {
            job_id,
            run_id,
            status: result.status,
            started_at: result.started_at,
            finished_at: result.finished_at,
            duration: result.duration(),
            exit_code: result.steps.last().map(|s| s.exit_code).unwrap_or(0),
            steps,
            artifacts: Vec::new(), // TODO: Collect artifacts
            metadata: HashMap::new(),
        }
    }
}
