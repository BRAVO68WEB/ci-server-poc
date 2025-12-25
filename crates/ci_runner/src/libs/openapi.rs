//! OpenAPI specification and documentation

use utoipa::OpenApi;
use utoipa::{
    openapi::security::{ApiKey, ApiKeyValue, SecurityScheme},
    Modify,
};

use crate::models::types::{JobEvent, JobResult, JobStatus};

#[derive(OpenApi)]
#[openapi(
    paths(
        crate::routes::api::health_check,
        crate::routes::api::readiness_check,
        crate::routes::api::metrics_handler,
        crate::routes::api::submit_job,
        crate::routes::api::get_job_status,
        crate::routes::api::list_jobs,
        crate::routes::api::get_job_logs,
        crate::routes::api::cancel_job,
        crate::routes::api::replay_job,
        crate::routes::api::retry_job,
        crate::routes::api::upload_artifact,
        crate::routes::api::download_artifact,
        crate::routes::api::openapi_json,
        crate::routes::api::scalar_docs,
        crate::libs::sse::stream_job_updates,
    ),
    components(schemas(
        JobEvent,
        JobResult,
        JobStatus,
        crate::models::types::RepositoryInfo,
        crate::models::types::TriggerInfo,
        crate::models::types::RunnerConfig,
        crate::models::types::Step,
        crate::models::types::StepResult,
        crate::models::types::LogEntry,
        crate::models::types::ArtifactInfo,
        crate::stores::memory::JobState,
        crate::stores::memory::JobStateStatus,
        crate::models::types::WhenCondition,
        crate::models::types::RetryPolicy,
        crate::routes::api::ReplayJobRequest,
    )),
    tags(
        (name = "health", description = "Health and readiness checks"),
        (name = "metrics", description = "Prometheus metrics"),
        (name = "jobs", description = "Job management endpoints"),
        (name = "artifacts", description = "Artifact management"),
    ),
    modifiers(&SecurityAddon),
)]
pub struct ApiDoc;

struct SecurityAddon;

impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        if let Some(components) = openapi.components.as_mut() {
            components.add_security_scheme(
                "api_key",
                SecurityScheme::ApiKey(ApiKey::Header(ApiKeyValue::new("X-API-Key"))),
            )
        }
    }
}

