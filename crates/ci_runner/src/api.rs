use actix_web::{HttpResponse, Responder, Result as ActixResult, web};
use chrono::Utc;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Clone)]
pub struct AppState {
    pub scheduler: Arc<crate::scheduler::JobScheduler>,
}

pub async fn health_check() -> ActixResult<impl Responder> {
    Ok(HttpResponse::Ok().json(json!({
        "status": "healthy",
        "timestamp": Utc::now()
    })))
}

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

pub async fn metrics_handler() -> ActixResult<impl Responder> {
    // TODO: Implement Prometheus metrics
    Ok(HttpResponse::Ok()
        .content_type("text/plain")
        .body("# CI Runner Metrics\n# TODO: Implement metrics\n"))
}

pub async fn cancel_job(
    path: web::Path<Uuid>,
    data: web::Data<AppState>,
) -> ActixResult<impl Responder> {
    let job_id = path.into_inner();

    match data.scheduler.cancel_job(job_id).await {
        Ok(_) => Ok(HttpResponse::NoContent().finish()),
        Err(_) => Ok(HttpResponse::NotFound().json(json!({
            "error": "Job not found"
        }))),
    }
}
