//! Server-Sent Events (SSE) for real-time job updates

use actix_web::{HttpResponse, Result as ActixResult, web, web::Bytes};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::broadcast;
use tokio::time::Duration;
use tracing::warn;
use uuid::Uuid;

pub struct JobEventBroadcaster {
    tx: broadcast::Sender<JobEventMessage>,
}

#[derive(Debug, Clone)]
pub struct JobEventMessage {
    pub job_id: Uuid,
    pub event_type: String,
    pub data: serde_json::Value,
}

impl JobEventBroadcaster {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(1000);
        Self { tx }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<JobEventMessage> {
        self.tx.subscribe()
    }

    pub fn broadcast(&self, message: JobEventMessage) {
        let _ = self.tx.send(message);
    }

    pub fn broadcast_job_status(&self, job_id: Uuid, status: &str, data: Option<serde_json::Value>) {
        self.broadcast(JobEventMessage {
            job_id,
            event_type: "status".to_string(),
            data: json!({
                "status": status,
                "job_id": job_id,
                "data": data
            }),
        });
    }

    pub fn broadcast_job_log(&self, job_id: Uuid, log_entry: &crate::models::types::LogEntry) {
        self.broadcast(JobEventMessage {
            job_id,
            event_type: "log".to_string(),
            data: json!({
                "timestamp": log_entry.timestamp,
                "level": format!("{:?}", log_entry.level).to_lowercase(),
                "step_name": log_entry.step_name,
                "message": log_entry.message,
                "sequence": log_entry.sequence,
            }),
        });
    }
}

impl Default for JobEventBroadcaster {
    fn default() -> Self {
        Self::new()
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/jobs/{job_id}/stream",
    tag = "jobs",
    params(
        ("job_id" = Uuid, Path, description = "Job ID to stream")
    ),
    responses(
        (status = 200, description = "SSE stream", content_type = "text/event-stream"),
        (status = 404, description = "Job not found"),
        (status = 401, description = "Unauthorized")
    ),
    security(
        ("api_key" = [])
    )
)]
pub async fn stream_job_updates(
    req: actix_web::HttpRequest,
    path: web::Path<Uuid>,
    data: web::Data<crate::routes::api::AppState>,
    broadcaster: web::Data<Arc<JobEventBroadcaster>>,
) -> ActixResult<HttpResponse> {
    // Check auth - verify API key if auth is enabled
    if let Some(ref auth_state) = data.auth_state {
        if let Some(api_key) = req.headers().get("X-API-Key") {
            if let Ok(key_str) = api_key.to_str() {
                if !auth_state.api_keys.contains(key_str) {
                    return Ok(HttpResponse::Unauthorized().json(json!({
                        "error": "Invalid API key"
                    })));
                }
            }
        } else {
            return Ok(HttpResponse::Unauthorized().json(json!({
                "error": "Missing API key"
            })));
        }
    }
    
    let job_id = path.into_inner();
    
    // Verify job exists
    if matches!(data.job_store.get_job(job_id).await, Ok(None) | Err(_)) {
        return Ok(HttpResponse::NotFound().json(json!({
            "error": "Job not found",
            "job_id": job_id
        })));
    }
    
    // Subscribe to events
    let mut rx = broadcaster.subscribe();
    
    // Create SSE response
    let stream = async_stream::stream! {
        // Send initial connection message
        yield Ok::<Bytes, actix_web::Error>(Bytes::from(format!("event: connected\ndata: {}\n\n", json!({
            "job_id": job_id,
            "message": "Connected to job stream"
        }))));
        
        // Send current job status
        if let Ok(Some(job)) = data.job_store.get_job(job_id).await {
            yield Ok::<Bytes, actix_web::Error>(Bytes::from(format!("event: status\ndata: {}\n\n", json!({
                "job_id": job.job_id,
                "status": format!("{:?}", job.status).to_lowercase(),
                "started_at": job.started_at,
                "finished_at": job.finished_at,
            }))));
        }
        
        // Stream events
        loop {
            tokio::select! {
                result = rx.recv() => {
                    match result {
                        Ok(msg) => {
                            if msg.job_id == job_id {
                                yield Ok::<Bytes, actix_web::Error>(Bytes::from(format!("event: {}\ndata: {}\n\n", msg.event_type, msg.data)));
                            }
                        }
                        Err(broadcast::error::RecvError::Closed) => {
                            yield Ok::<Bytes, actix_web::Error>(Bytes::from("event: closed\ndata: {}\n\n"));
                            break;
                        }
                        Err(broadcast::error::RecvError::Lagged(skipped)) => {
                            warn!(job_id = %job_id, skipped = skipped, "SSE stream lagged");
                            yield Ok::<Bytes, actix_web::Error>(Bytes::from(format!("event: lagged\ndata: {}\n\n", json!({
                                "skipped": skipped,
                                "message": "Stream lagged, some events may have been missed"
                            }))));
                        }
                    }
                }
                _ = tokio::time::sleep(Duration::from_secs(30)) => {
                    // Send heartbeat to keep connection alive
                    yield Ok::<Bytes, actix_web::Error>(Bytes::from(": heartbeat\n\n"));
                }
            }
        }
    };
    
    Ok(HttpResponse::Ok()
        .content_type("text/event-stream")
        .append_header(("Cache-Control", "no-cache"))
        .append_header(("Connection", "keep-alive"))
        .append_header(("X-Accel-Buffering", "no"))
        .streaming(stream))
}
