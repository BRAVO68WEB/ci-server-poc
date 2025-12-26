//! HTTP route handlers for the CI Runner API

use actix_web::{web, HttpResponse, Responder, Result as ActixResult};
use chrono::Utc;
use prometheus::Encoder;
use serde_json::json;
use tracing::{info, error};
use std::sync::Arc;
use uuid::Uuid;
use crate::models::types::{JobEvent, JobResult};
use crate::stores::memory::{JobState, JobStateStatus};

#[utoipa::path(
    get,
    path = "/api-docs/openapi.json",
    tag = "docs",
    responses(
        (status = 200, description = "OpenAPI specification", content_type = "application/json")
    )
)]
pub async fn openapi_json() -> ActixResult<impl Responder> {
    use utoipa::OpenApi;
    Ok(HttpResponse::Ok()
        .content_type("application/json")
        .json(serde_json::to_value(crate::libs::openapi::ApiDoc::openapi()).unwrap_or_default()))
}

#[utoipa::path(
    get,
    path = "/api-docs",
    tag = "docs",
    responses(
        (status = 200, description = "Scalar API documentation UI", content_type = "text/html")
    )
)]
pub async fn scalar_docs() -> ActixResult<impl Responder> {
    Ok(crate::libs::scalar::scalar_ui().await)
}


pub struct AppState {
    pub scheduler: Arc<crate::services::scheduler::JobScheduler>,
    pub job_store: Arc<crate::stores::memory::JobStore>,
    pub artifact_store: Arc<dyn crate::stores::artifact_trait::ArtifactStorage>,
    pub auth_state: Option<Arc<crate::middleware::auth::AuthState>>,
    pub job_handler: Arc<dyn Fn(JobEvent) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<JobResult, crate::models::error::ExecutionError>> + Send>> + Send + Sync>,
    pub event_broadcaster: Option<Arc<crate::libs::sse::JobEventBroadcaster>>,
}

// Helper function to check auth
async fn check_auth(req: &actix_web::HttpRequest, auth_state: &Option<Arc<crate::middleware::auth::AuthState>>) -> Result<(), HttpResponse> {
    if let Some(auth) = auth_state {
        if auth.api_keys.is_empty() {
            return Ok(()); // Auth disabled
        }
        
        let api_key = req.headers()
            .get("X-API-Key")
            .and_then(|h| h.to_str().ok())
            .map(|s| s.to_string());
        
        if let Some(key) = api_key {
            if !auth.api_keys.contains(&key) {
                return Err(HttpResponse::Unauthorized().json(json!({
                    "error": "Invalid API key"
                })));
            }
            
            // Check rate limit
            if !auth.rate_limiter.check(&key).await {
                return Err(HttpResponse::TooManyRequests().json(json!({
                    "error": "Rate limit exceeded"
                })));
            }
        } else {
            return Err(HttpResponse::Unauthorized().json(json!({
                "error": "Missing API key header: X-API-Key"
            })));
        }
    }
    Ok(())
}

#[derive(serde::Deserialize, utoipa::ToSchema)]
pub struct ReplayJobRequest {
    #[serde(default)]
    pub modify_config: Option<serde_json::Value>,
    #[serde(default)]
    pub new_run_id: Option<Uuid>,
}

#[utoipa::path(
    post,
    path = "/api/v1/jobs/{job_id}/replay",
    tag = "jobs",
    params(
        ("job_id" = Uuid, Path, description = "Job ID to replay")
    ),
    request_body = ReplayJobRequest,
    responses(
        (status = 202, description = "Job replayed successfully"),
        (status = 404, description = "Job not found"),
        (status = 401, description = "Unauthorized")
    ),
    security(
        ("api_key" = [])
    )
)]
pub async fn replay_job(
    req: actix_web::HttpRequest,
    path: web::Path<Uuid>,
    data: web::Data<AppState>,
    body: Option<web::Json<ReplayJobRequest>>,
) -> ActixResult<impl Responder> {
    // Check auth
    if let Err(resp) = check_auth(&req, &data.auth_state).await {
        return Ok(resp);
    }
    
    let job_id = path.into_inner();
    
    // Get original job
    let original_job = match data.job_store.get_job(job_id).await {
        Some(job) => job,
        None => {
            return Ok(HttpResponse::NotFound().json(json!({
                "error": "Job not found",
                "job_id": job_id
            })));
        }
    };
    
    // Create new job event based on original
    let mut new_event = original_job.event.clone();
    
    // Generate new job_id and run_id
    let new_job_id = Uuid::new_v4();
    let new_run_id = body.as_ref()
        .and_then(|b| b.new_run_id)
        .unwrap_or_else(Uuid::new_v4);
    
    new_event.job_id = new_job_id;
    new_event.run_id = new_run_id;
    new_event.timestamp = chrono::Utc::now();
    
    // Apply config modifications if provided
    if let Some(ref request) = body {
        if request.modify_config.is_some() {
            // Merge modifications into config_path (would need to load and modify config)
            // For now, we'll just store the modifications in metadata
            info!(original_job_id = %job_id, new_job_id = %new_job_id, "Replaying job with modifications");
        }
    }
    
    // Submit the new job
    let handler = Arc::clone(&data.job_handler);
    let scheduler = Arc::clone(&data.scheduler);
    let store = Arc::clone(&data.job_store);
    let broadcaster = data.event_broadcaster.clone();
    
    match scheduler.schedule(new_event.clone(), move |event: JobEvent| {
        let handler = Arc::clone(&handler);
        let store = Arc::clone(&store);
        let broadcaster_clone = broadcaster.clone();
        async move {
            let _ = store.update_job_status(event.job_id, JobStateStatus::Running).await;
            crate::utils::metrics::get_metrics().record_job_start();
            
            if let Some(ref b) = broadcaster_clone {
                b.broadcast_job_status(event.job_id, "running", None);
            }
            
            let result = handler(event.clone()).await;
            
            match &result {
                Ok(job_result) => {
                    let _ = store.set_job_result(event.job_id, job_result.clone()).await;
                    let duration = job_result.duration().as_secs_f64();
                    let status = format!("{:?}", job_result.status).to_lowercase();
                    crate::utils::metrics::get_metrics().record_job_complete(&status, duration);
                    
                    if let Some(ref b) = broadcaster_clone {
                        b.broadcast_job_status(event.job_id, &status, Some(json!({
                            "duration": duration,
                        })));
                    }
                }
                Err(e) => {
                    let error_msg = e.to_string();
                    let _ = store.set_job_error(event.job_id, error_msg.clone()).await;
                    crate::utils::metrics::get_metrics().record_error("execution_error");
                    crate::utils::metrics::get_metrics().record_job_complete("failed", 0.0);
                    
                    if let Some(ref b) = broadcaster_clone {
                        b.broadcast_job_status(event.job_id, "failed", Some(json!({
                            "error": error_msg,
                        })));
                    }
                }
            }
            
            result
        }
    }).await {
        Ok(_) => {
            info!(original_job_id = %job_id, new_job_id = %new_job_id, "Job replayed successfully");
            Ok(HttpResponse::Accepted().json(json!({
                "status": "replayed",
                "original_job_id": job_id,
                "new_job_id": new_job_id,
                "new_run_id": new_run_id,
                "message": "Job replayed successfully"
            })))
        }
        Err(e) => {
            error!(original_job_id = %job_id, error = %e, "Failed to replay job");
            Ok(HttpResponse::BadRequest().json(json!({
                "error": format!("Failed to replay job: {}", e),
                "original_job_id": job_id
            })))
        }
    }
}

#[utoipa::path(
    post,
    path = "/api/v1/jobs/{job_id}/retry",
    tag = "jobs",
    params(
        ("job_id" = Uuid, Path, description = "Job ID to retry")
    ),
    responses(
        (status = 202, description = "Job retry initiated"),
        (status = 400, description = "Job cannot be retried (not failed)"),
        (status = 404, description = "Job not found"),
        (status = 401, description = "Unauthorized")
    ),
    security(
        ("api_key" = [])
    )
)]
pub async fn retry_job(
    req: actix_web::HttpRequest,
    path: web::Path<Uuid>,
    data: web::Data<AppState>,
) -> ActixResult<impl Responder> {
    // Check auth
    if let Err(resp) = check_auth(&req, &data.auth_state).await {
        return Ok(resp);
    }
    
    let job_id = path.into_inner();
    
    // Get original job
    let original_job = match data.job_store.get_job(job_id).await {
        Some(job) => job,
        None => {
            return Ok(HttpResponse::NotFound().json(json!({
                "error": "Job not found",
                "job_id": job_id
            })));
        }
    };
    
    // Only retry failed jobs
    if original_job.status != JobStateStatus::Failed {
        return Ok(HttpResponse::BadRequest().json(json!({
            "error": "Job can only be retried if it failed",
            "job_id": job_id,
            "current_status": format!("{:?}", original_job.status).to_lowercase()
        })));
    }
    
    // Create new job event (same as replay but specifically for retries)
    let mut new_event = original_job.event.clone();
    let new_job_id = Uuid::new_v4();
    let new_run_id = Uuid::new_v4();
    
    new_event.job_id = new_job_id;
    new_event.run_id = new_run_id;
    new_event.timestamp = chrono::Utc::now();
    
    // Submit the retry job
    let handler = Arc::clone(&data.job_handler);
    let scheduler = Arc::clone(&data.scheduler);
    let store = Arc::clone(&data.job_store);
    let broadcaster = data.event_broadcaster.clone();
    
    match scheduler.schedule(new_event.clone(), move |event: JobEvent| {
        let handler = Arc::clone(&handler);
        let store = Arc::clone(&store);
        let broadcaster_clone = broadcaster.clone();
        async move {
            let _ = store.update_job_status(event.job_id, JobStateStatus::Running).await;
            crate::utils::metrics::get_metrics().record_job_start();
            
            if let Some(ref b) = broadcaster_clone {
                b.broadcast_job_status(event.job_id, "running", None);
            }
            
            let result = handler(event.clone()).await;
            
            match &result {
                Ok(job_result) => {
                    let _ = store.set_job_result(event.job_id, job_result.clone()).await;
                    let duration = job_result.duration().as_secs_f64();
                    let status = format!("{:?}", job_result.status).to_lowercase();
                    crate::utils::metrics::get_metrics().record_job_complete(&status, duration);
                    
                    if let Some(ref b) = broadcaster_clone {
                        b.broadcast_job_status(event.job_id, &status, Some(json!({
                            "duration": duration,
                        })));
                    }
                }
                Err(e) => {
                    let error_msg = e.to_string();
                    let _ = store.set_job_error(event.job_id, error_msg.clone()).await;
                    crate::utils::metrics::get_metrics().record_error("execution_error");
                    crate::utils::metrics::get_metrics().record_job_complete("failed", 0.0);
                    
                    if let Some(ref b) = broadcaster_clone {
                        b.broadcast_job_status(event.job_id, "failed", Some(json!({
                            "error": error_msg,
                        })));
                    }
                }
            }
            
            result
        }
    }).await {
        Ok(_) => {
            info!(original_job_id = %job_id, new_job_id = %new_job_id, "Job retry initiated");
            Ok(HttpResponse::Accepted().json(json!({
                "status": "retry_initiated",
                "original_job_id": job_id,
                "new_job_id": new_job_id,
                "new_run_id": new_run_id,
                "message": "Job retry initiated successfully"
            })))
        }
        Err(e) => {
            error!(original_job_id = %job_id, error = %e, "Failed to retry job");
            Ok(HttpResponse::BadRequest().json(json!({
                "error": format!("Failed to retry job: {}", e),
                "original_job_id": job_id
            })))
        }
    }
}

#[utoipa::path(
    get,
    path = "/health",
    tag = "health",
    responses(
        (status = 200, description = "Service is healthy")
    )
)]
pub async fn health_check() -> ActixResult<impl Responder> {
    Ok(HttpResponse::Ok().json(json!({
        "status": "healthy",
        "timestamp": Utc::now()
    })))
}

#[utoipa::path(
    get,
    path = "/ready",
    tag = "health",
    responses(
        (status = 200, description = "Service readiness status")
    )
)]
pub async fn readiness_check(data: web::Data<AppState>) -> ActixResult<impl Responder> {
    let active_jobs = data.scheduler.active_jobs().await;
    let status = if active_jobs < 100 {
        "ready"
    } else {
        "not_ready"
    };

    Ok(HttpResponse::Ok().json(json!({
        "status": status,
        "active_jobs": active_jobs,
        "timestamp": Utc::now()
    })))
}

#[utoipa::path(
    get,
    path = "/metrics",
    tag = "metrics",
    responses(
        (status = 200, description = "Prometheus metrics")
    )
)]
pub async fn metrics_handler() -> ActixResult<impl Responder> {
    use prometheus::TextEncoder;
    
    let encoder = TextEncoder::new();
    let metric_families = prometheus::gather();
    
    // Create a mutable buffer for encoding
    let mut buffer = Vec::new();
    
    match encoder.encode(&metric_families, &mut buffer) {
        Ok(_) => {
            // Ensure metrics are initialized (this ensures they're registered)
            let _ = crate::utils::metrics::Metrics::init();
            
            Ok(HttpResponse::Ok()
                .content_type("text/plain; version=0.0.4")
                .body(buffer))
        }
        Err(e) => {
            error!("Failed to encode metrics: {}", e);
            Ok(HttpResponse::InternalServerError()
                .body(format!("Failed to encode metrics: {}", e)))
        }
    }
}

#[utoipa::path(
    post,
    path = "/api/v1/jobs",
    tag = "jobs",
    request_body = JobEvent,
    responses(
        (status = 202, description = "Job submitted successfully"),
        (status = 401, description = "Unauthorized"),
        (status = 429, description = "Rate limit exceeded")
    ),
    security(
        ("api_key" = [])
    )
)]
pub async fn submit_job(
    req: actix_web::HttpRequest,
    data: web::Data<AppState>,
    job_event: web::Json<JobEvent>,
) -> ActixResult<impl Responder> {
    use crate::utils::metrics::get_metrics;
    
    // Check auth
    if let Err(resp) = check_auth(&req, &data.auth_state).await {
        return Ok(resp);
    }
    
    let job_event = job_event.into_inner();
    let job_id = job_event.job_id;
    let start_time = std::time::Instant::now();

    info!(job_id = %job_id, "Received job submission via HTTP");

    // Create job in store
    if let Err(e) = data.job_store.create_job(job_event.clone()).await {
        error!(job_id = %job_id, error = %e, "Failed to create job in store");
        get_metrics().record_error("store_error");
        return Ok(HttpResponse::InternalServerError().json(json!({
            "error": format!("Failed to create job: {}", e),
            "job_id": job_id
        })));
    }

    // Schedule the job asynchronously
    let handler = Arc::clone(&data.job_handler);
    let scheduler = Arc::clone(&data.scheduler);
    let store = Arc::clone(&data.job_store);
    let broadcaster = data.event_broadcaster.clone();

    match scheduler.schedule(job_event.clone(), move |event: JobEvent| {
        let handler = Arc::clone(&handler);
        let store = Arc::clone(&store);
        let broadcaster_clone = broadcaster.clone();
        async move {
            // Update status to running
            let _ = store.update_job_status(event.job_id, JobStateStatus::Running).await;
            get_metrics().record_job_start();
            
            // Broadcast job started event
            if let Some(ref b) = broadcaster_clone {
                b.broadcast_job_status(event.job_id, "running", None);
            }
            
            let result = handler(event.clone()).await;
            
            // Update store with result
            match &result {
                Ok(job_result) => {
                    let _ = store.set_job_result(event.job_id, job_result.clone()).await;
                    let duration = job_result.duration().as_secs_f64();
                    let status = format!("{:?}", job_result.status).to_lowercase();
                    get_metrics().record_job_complete(&status, duration);
                    
                    // Broadcast job completed event
                    if let Some(ref b) = broadcaster_clone {
                        b.broadcast_job_status(event.job_id, &status, Some(json!({
                            "duration": duration,
                            "exit_code": job_result.steps.last().map(|s| s.exit_code).unwrap_or(0),
                        })));
                    }
                    // Note: Artifacts are collected in from_result call
                }
                Err(e) => {
                    let error_msg = e.to_string();
                    let _ = store.set_job_error(event.job_id, error_msg.clone()).await;
                    get_metrics().record_error("execution_error");
                    get_metrics().record_job_complete("failed", 0.0);
                    
                    // Broadcast job failed event
                    if let Some(ref b) = broadcaster_clone {
                        b.broadcast_job_status(event.job_id, "failed", Some(json!({
                            "error": error_msg,
                        })));
                    }
                }
            }
            
            result
        }
    }).await {
        Ok(_) => {
            let queue_wait = start_time.elapsed().as_secs_f64();
            get_metrics().record_queue_wait(queue_wait);
            
            info!(job_id = %job_id, "Job scheduled successfully");
            Ok(HttpResponse::Accepted().json(json!({
                "status": "accepted",
                "job_id": job_id,
                "message": "Job submitted successfully"
            })))
        }
        Err(e) => {
            error!(job_id = %job_id, error = %e, "Failed to schedule job");
            let _ = data.job_store.set_job_error(job_id, e.to_string()).await;
            get_metrics().record_error("schedule_error");
            Ok(HttpResponse::BadRequest().json(json!({
                "error": format!("Failed to schedule job: {}", e),
                "job_id": job_id
            })))
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/jobs/{job_id}",
    tag = "jobs",
    params(
        ("job_id" = Uuid, Path, description = "Job ID")
    ),
    responses(
        (status = 200, description = "Job details", body = JobState),
        (status = 404, description = "Job not found"),
        (status = 401, description = "Unauthorized")
    ),
    security(
        ("api_key" = [])
    )
)]
pub async fn get_job_status(
    req: actix_web::HttpRequest,
    path: web::Path<Uuid>,
    data: web::Data<AppState>,
) -> ActixResult<impl Responder> {
    // Check auth
    if let Err(resp) = check_auth(&req, &data.auth_state).await {
        return Ok(resp);
    }
    
    let job_id = path.into_inner();

    match data.job_store.get_job(job_id).await {
        Some(job) => {
            Ok(HttpResponse::Ok().json(json!({
                "job_id": job.job_id,
                "run_id": job.run_id,
                "status": format!("{:?}", job.status).to_lowercase(),
                "started_at": job.started_at,
                "finished_at": job.finished_at,
                "repository": {
                    "owner": job.event.repository.owner,
                    "name": job.event.repository.name,
                    "commit_sha": job.event.repository.commit_sha,
                    "ref_name": job.event.repository.ref_name,
                },
                "trigger": {
                    "event_type": format!("{:?}", job.event.trigger.event_type).to_lowercase(),
                    "actor": job.event.trigger.actor,
                },
                "result": job.result.as_ref().map(|r| serde_json::json!({
                    "status": format!("{:?}", r.status).to_lowercase(),
                    "steps": r.steps.iter().map(|s| {
                        let step_duration = s.finished_at.signed_duration_since(s.started_at);
                        serde_json::json!({
                            "name": s.name,
                            "step_type": format!("{:?}", s.step_type).to_lowercase(),
                            "exit_code": s.exit_code,
                            "duration_secs": step_duration.num_seconds(),
                            "started_at": s.started_at,
                            "finished_at": s.finished_at,
                        })
                    }).collect::<Vec<_>>(),
                    "started_at": r.started_at,
                    "finished_at": r.finished_at,
                    "duration_secs": r.duration().as_secs(),
                })),
                "error": job.error,
                "artifacts": job.artifacts.iter().map(|a| serde_json::json!({
                    "name": a.name,
                    "size": a.size,
                    "checksum": a.checksum,
                    "url": a.url,
                })).collect::<Vec<_>>(),
            })))
        }
        None => Ok(HttpResponse::NotFound().json(json!({
            "error": "Job not found",
            "job_id": job_id
        }))),
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/jobs",
    tag = "jobs",
    params(
        ("limit" = Option<usize>, Query, description = "Maximum number of jobs to return"),
        ("offset" = Option<usize>, Query, description = "Number of jobs to skip"),
        ("status" = Option<String>, Query, description = "Filter by status (pending/running/completed/failed/cancelled)")
    ),
    responses(
        (status = 200, description = "List of jobs"),
        (status = 401, description = "Unauthorized")
    ),
    security(
        ("api_key" = [])
    )
)]
pub async fn list_jobs(
    req: actix_web::HttpRequest,
    data: web::Data<AppState>,
    query: web::Query<ListJobsQuery>,
) -> ActixResult<impl Responder> {
    // Check auth
    if let Err(resp) = check_auth(&req, &data.auth_state).await {
        return Ok(resp);
    }
    
    let limit = query.limit.unwrap_or(50).min(100);
    let offset = query.offset.unwrap_or(0);
    
    let status_filter = query.status.as_ref().and_then(|s| {
        match s.as_str() {
            "pending" => Some(JobStateStatus::Pending),
            "running" => Some(JobStateStatus::Running),
            "completed" => Some(JobStateStatus::Completed),
            "failed" => Some(JobStateStatus::Failed),
            "cancelled" => Some(JobStateStatus::Cancelled),
            _ => None,
        }
    });

    let jobs = data.job_store.list_jobs(limit, offset, status_filter).await;
    let total = data.job_store.count_jobs(status_filter).await;

    Ok(HttpResponse::Ok().json(json!({
        "jobs": jobs.iter().map(|job| json!({
            "job_id": job.job_id,
            "run_id": job.run_id,
            "status": format!("{:?}", job.status).to_lowercase(),
            "started_at": job.started_at,
            "finished_at": job.finished_at,
            "repository": {
                "owner": job.event.repository.owner,
                "name": job.event.repository.name,
            },
        })).collect::<Vec<_>>(),
        "pagination": {
            "limit": limit,
            "offset": offset,
            "total": total,
        }
    })))
}

#[derive(serde::Deserialize)]
pub struct ListJobsQuery {
    pub limit: Option<usize>,
    pub offset: Option<usize>,
    pub status: Option<String>,
}

#[utoipa::path(
    get,
    path = "/api/v1/jobs/{job_id}/logs",
    tag = "jobs",
    params(
        ("job_id" = Uuid, Path, description = "Job ID"),
        ("limit" = Option<usize>, Query, description = "Maximum number of log entries to return")
    ),
    responses(
        (status = 200, description = "Job logs"),
        (status = 404, description = "Job not found"),
        (status = 401, description = "Unauthorized")
    ),
    security(
        ("api_key" = [])
    )
)]
pub async fn get_job_logs(
    req: actix_web::HttpRequest,
    path: web::Path<Uuid>,
    data: web::Data<AppState>,
    query: web::Query<LogsQuery>,
) -> ActixResult<impl Responder> {
    // Check auth
    if let Err(resp) = check_auth(&req, &data.auth_state).await {
        return Ok(resp);
    }
    
    let job_id = path.into_inner();
    let limit = query.limit;

    let logs = data.job_store.get_logs(job_id, limit).await;

    if logs.is_empty() {
        // Check if job exists
        if data.job_store.get_job(job_id).await.is_none() {
            return Ok(HttpResponse::NotFound().json(json!({
                "error": "Job not found",
                "job_id": job_id
            })));
        }
    }

    Ok(HttpResponse::Ok().json(json!({
        "job_id": job_id,
        "logs": logs.iter().map(|log| json!({
            "timestamp": log.timestamp,
            "level": format!("{:?}", log.level).to_lowercase(),
            "step_name": log.step_name,
            "message": log.message,
            "sequence": log.sequence,
        })).collect::<Vec<_>>(),
        "count": logs.len(),
    })))
}

#[derive(serde::Deserialize)]
pub struct LogsQuery {
    pub limit: Option<usize>,
}

#[utoipa::path(
    post,
    path = "/api/v1/jobs/{job_id}/cancel",
    tag = "jobs",
    params(
        ("job_id" = Uuid, Path, description = "Job ID")
    ),
    responses(
        (status = 200, description = "Job cancelled"),
        (status = 404, description = "Job not found"),
        (status = 401, description = "Unauthorized")
    ),
    security(
        ("api_key" = [])
    )
)]
pub async fn cancel_job(
    req: actix_web::HttpRequest,
    path: web::Path<Uuid>,
    data: web::Data<AppState>,
) -> ActixResult<impl Responder> {
    // Check auth
    if let Err(resp) = check_auth(&req, &data.auth_state).await {
        return Ok(resp);
    }
    
    let job_id = path.into_inner();

    match data.scheduler.cancel_job(job_id).await {
        Ok(_) => {
            let _ = data.job_store.update_job_status(job_id, JobStateStatus::Cancelled).await;
            Ok(HttpResponse::NoContent().finish())
        }
        Err(_) => Ok(HttpResponse::NotFound().json(json!({
            "error": "Job not found"
        }))),
    }
}

#[utoipa::path(
    post,
    path = "/api/v1/jobs/{job_id}/artifacts/{artifact_name}",
    tag = "artifacts",
    params(
        ("job_id" = Uuid, Path, description = "Job ID"),
        ("artifact_name" = String, Path, description = "Artifact name")
    ),
    request_body(content = Vec<u8>, description = "Artifact file content"),
    responses(
        (status = 201, description = "Artifact uploaded"),
        (status = 404, description = "Job not found"),
        (status = 401, description = "Unauthorized")
    ),
    security(
        ("api_key" = [])
    )
)]
pub async fn upload_artifact(
    req: actix_web::HttpRequest,
    path: web::Path<(Uuid, String)>,
    data: web::Data<AppState>,
    body: web::Bytes,
) -> ActixResult<impl Responder> {
    // Check auth
    if let Err(resp) = check_auth(&req, &data.auth_state).await {
        return Ok(resp);
    }
    
    let (job_id, artifact_name) = path.into_inner();
    
    // Verify job exists
    if data.job_store.get_job(job_id).await.is_none() {
        return Ok(HttpResponse::NotFound().json(json!({
            "error": "Job not found",
            "job_id": job_id
        })));
    }

    match data.artifact_store.upload_artifact(job_id, artifact_name.clone(), body.to_vec()).await {
        Ok(artifact_info) => {
            Ok(HttpResponse::Created().json(json!({
                "status": "uploaded",
                "artifact": artifact_info
            })))
        }
        Err(e) => {
            error!(job_id = %job_id, artifact = %artifact_name, error = %e, "Failed to upload artifact");
            Ok(HttpResponse::InternalServerError().json(json!({
                "error": format!("Failed to upload artifact: {}", e),
                "job_id": job_id
            })))
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/jobs/{job_id}/artifacts/{artifact_name}",
    tag = "artifacts",
    params(
        ("job_id" = Uuid, Path, description = "Job ID"),
        ("artifact_name" = String, Path, description = "Artifact name")
    ),
    responses(
        (status = 200, description = "Artifact file", content_type = "application/octet-stream"),
        (status = 404, description = "Artifact not found"),
        (status = 401, description = "Unauthorized")
    ),
    security(
        ("api_key" = [])
    )
)]
pub async fn download_artifact(
    req: actix_web::HttpRequest,
    path: web::Path<(Uuid, String)>,
    data: web::Data<AppState>,
) -> ActixResult<impl Responder> {
    // Check auth
    if let Err(resp) = check_auth(&req, &data.auth_state).await {
        return Ok(resp);
    }
    
    let (job_id, artifact_name) = path.into_inner();

    match data.artifact_store.download_artifact(job_id, &artifact_name).await {
        Ok(data) => {
            Ok(HttpResponse::Ok()
                .content_type("application/octet-stream")
                .append_header(("Content-Disposition", format!("attachment; filename=\"{}\"", artifact_name)))
                .body(data))
        }
        Err(e) => {
            if e.to_string().contains("not found") {
                Ok(HttpResponse::NotFound().json(json!({
                    "error": "Artifact not found",
                    "job_id": job_id,
                    "artifact": artifact_name
                })))
            } else {
                error!(job_id = %job_id, artifact = %artifact_name, error = %e, "Failed to download artifact");
                Ok(HttpResponse::InternalServerError().json(json!({
                    "error": format!("Failed to download artifact: {}", e),
                    "job_id": job_id
                })))
            }
        }
    }
}

pub async fn list_artifacts(
    req: actix_web::HttpRequest,
    path: web::Path<Uuid>,
    data: web::Data<AppState>,
) -> ActixResult<impl Responder> {
    // Check auth
    if let Err(resp) = check_auth(&req, &data.auth_state).await {
        return Ok(resp);
    }
    
    let job_id = path.into_inner();

    // Verify job exists
    if data.job_store.get_job(job_id).await.is_none() {
        return Ok(HttpResponse::NotFound().json(json!({
            "error": "Job not found",
            "job_id": job_id
        })));
    }

    let artifacts = data.artifact_store.list_artifacts(job_id).await;

    Ok(HttpResponse::Ok().json(json!({
        "job_id": job_id,
        "artifacts": artifacts,
        "count": artifacts.len(),
    })))
}
