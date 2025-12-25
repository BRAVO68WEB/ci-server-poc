use actix_web::web;
use ci_runner::{routes, config, core};
use std::sync::Arc;
use std::time::Duration;
use tokio::signal;
use tracing::{error, info, warn};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::filter::EnvFilter::from_default_env())
        .json()
        .init();

    info!("Starting CI Runner");

    // Load configuration
    let config_path = std::env::var("CI_CONFIG")
        .ok()
        .or_else(|| Some("/etc/ci-runner/config.yaml".to_string()))
        .unwrap();

    let config = config::Config::load(Some(&config_path)).map_err(|e| {
        error!("Failed to load config: {}", e);
        e
    })?;

    info!("Configuration loaded");

    // Connect to Docker
    let docker = bollard::Docker::connect_with_socket(
        &config.executor.docker.socket,
        120,
        bollard::API_DEFAULT_VERSION,
    )
    .map_err(|e| {
        error!("Failed to connect to Docker: {}", e);
        e
    })?;

    // Load auth token
    let auth_token = tokio::fs::read_to_string(&config.git_server.service_token_path)
        .await
        .map_err(|e| {
            error!("Failed to read auth token: {}", e);
            e
        })?;
    let auth_token = auth_token.trim().to_string();

    // Initialize metrics
    ci_runner::utils::metrics::Metrics::init();
    info!("Metrics initialized");

    // Initialize job store
    let job_store = Arc::new(ci_runner::stores::memory::JobStore::new(config.store.max_history));
    info!("Job store initialized");

    // Initialize artifact store
    let artifact_store = Arc::new(ci_runner::stores::artifacts::ArtifactStore::new(
        config.executor.cache_root.join("artifacts"),
    ));
    artifact_store.initialize().await.map_err(|e| {
        error!("Failed to initialize artifact store: {}", e);
        e
    })?;
    info!("Artifact store initialized");

    // Initialize SSE event broadcaster
    let event_broadcaster = Arc::new(ci_runner::libs::sse::JobEventBroadcaster::new());
    info!("Event broadcaster initialized");

    // Initialize auth state
    let mut api_keys = std::collections::HashSet::new();
    if config.auth.enabled {
        // Load API keys from config
        for key in &config.auth.api_keys {
            api_keys.insert(key.clone());
        }
        
        // Load API keys from file if specified
        if let Some(ref api_key_file) = config.auth.api_key_file {
            if let Ok(contents) = tokio::fs::read_to_string(api_key_file).await {
                for line in contents.lines() {
                    let key = line.trim();
                    if !key.is_empty() {
                        api_keys.insert(key.to_string());
                    }
                }
            }
        }
        
        info!("Authentication enabled with {} API keys", api_keys.len());
    } else {
        info!("Authentication disabled");
    }

    let auth_state = web::Data::new(ci_runner::middleware::auth::AuthState {
        api_keys: Arc::new(api_keys),
        rate_limiter: Arc::new(ci_runner::middleware::auth::RateLimiter::new(
            config.auth.rate_limit_per_minute,
        )),
    });

    // Initialize application
    let app = core::App::initialize(&config).await?;

    // Create job handler
    let docker_clone = docker.clone();
    let workspace_manager_clone = Arc::clone(&app.workspace_manager);
    let config_clone = config.clone();
    let auth_token_clone = auth_token.clone();

    let job_handler = move |job_event: ci_runner::JobEvent| {
        let docker = docker_clone.clone();
        let workspace_manager = workspace_manager_clone.clone();
        let config = config_clone.clone();
        let auth_token = auth_token_clone.clone();

        async move {
            use ci_runner::services::cloner::{RepositoryCloner, ServiceAuth, TokenType};
            use ci_runner::services::event_publisher::EventPublisher;
            use ci_runner::services::executor::{JobExecutor, LogStreamerTrait};
            use ci_runner::services::log_streamer::LogStreamer;
            use ci_runner::services::parser::TaskParser;
            use ci_runner::models::types::{JobCompletionEvent, JobContext};

            let job_id = job_event.job_id;
            let run_id = job_event.run_id;

            info!(job_id = %job_id, "Processing job");

            // Initialize components for this job
            let cloner = Arc::new(RepositoryCloner::new(
                config.executor.workspace_root.clone(),
                config.executor.timeouts.git_clone,
                Some(1),                // Shallow clone
                true,                   // LFS enabled
            ));

            let parser = Arc::new(TaskParser::new(1024 * 1024)); // 1MB max

            let log_streamer = Arc::new(LogStreamer::new(
                config.log_streamer.clone(),
                auth_token.clone(),
            ));
            log_streamer.clone().start_background_flusher();

            let event_publisher = Arc::new(EventPublisher::new(
                config.git_server.clone(),
                auth_token.clone(),
            ));

            let executor_config = core::App::create_executor_config(&config);

            // Clone repository
            let auth = ServiceAuth {
                token_type: TokenType::Bearer,
                token: secrecy::SecretString::new(auth_token.into()),
                expiry: None,
            };

            let workspace_path = (*cloner)
                .clone(job_id, &job_event.repository, &auth)
                .await?;

            // Cleanup workspace on exit
            let workspace_manager_cleanup = workspace_manager.clone();
            let cleanup_workspace = move || {
                let workspace_manager = workspace_manager_cleanup.clone();
                let job_id = job_id;
                async move {
                    if let Err(e) = workspace_manager.cleanup_workspace(job_id).await {
                        warn!(job_id = %job_id, error = %e, "Failed to cleanup workspace");
                    }
                }
            };

            // Parse runner.yaml
            let runner_config = (*parser).parse(&workspace_path).await?;

            // Create job context
            let job_context = JobContext {
                job_id,
                run_id,
                workspace_path: workspace_path.clone(),
                config: runner_config,
                repository: job_event.repository,
                trigger: job_event.trigger,
            };

            // Execute job
            let executor = JobExecutor::new(docker, executor_config)
                .with_log_streamer(log_streamer.clone() as Arc<dyn LogStreamerTrait>);

            let result = executor.execute(job_context.clone()).await?;

            // Publish completion event
            let completion_event = JobCompletionEvent::from_result(
                job_id,
                run_id,
                &result,
                Some(&job_context.workspace_path),
            ).await;

            if let Err(e) = event_publisher.publish(completion_event).await {
                warn!(job_id = %job_id, error = %e, "Failed to publish completion event");
                // Don't fail the job if publishing fails
            }

            // Cleanup workspace
            cleanup_workspace().await;

            Ok::<ci_runner::models::types::JobResult, ci_runner::models::error::ExecutionError>(result)
        }
    };

    // Start HTTP server
    use actix_web::{App as ActixApp, HttpServer};
    let scheduler_for_server = Arc::clone(&app.scheduler);
    let job_store_for_server = Arc::clone(&job_store);
    let artifact_store_for_server = Arc::clone(&artifact_store);
    let auth_state_for_server = auth_state.clone();
    let event_broadcaster_for_server = Arc::clone(&event_broadcaster);
    
    // Wrap job handler in a boxed closure that matches the trait object signature
    let job_handler_wrapper: Arc<dyn Fn(ci_runner::JobEvent) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ci_runner::models::types::JobResult, ci_runner::models::error::ExecutionError>> + Send>> + Send + Sync> = 
        Arc::new(move |event: ci_runner::JobEvent| -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ci_runner::models::types::JobResult, ci_runner::models::error::ExecutionError>> + Send>> {
            let future = job_handler(event);
            Box::pin(future)
        });
    
    let server_addr = format!("{}:{}", config.server.host, config.server.port);

    info!("Starting HTTP server on {}", server_addr);
    let server_handle = tokio::spawn(async move {
        HttpServer::new(move || {
            let app_state = routes::api::AppState {
                scheduler: Arc::clone(&scheduler_for_server),
                job_store: Arc::clone(&job_store_for_server),
                artifact_store: Arc::clone(&artifact_store_for_server),
                auth_state: Some(Arc::new(auth_state_for_server.get_ref().clone())),
                job_handler: Arc::clone(&job_handler_wrapper),
                event_broadcaster: Some(Arc::clone(&event_broadcaster_for_server)),
            };
            
            
            
            ActixApp::new()
                .app_data(web::Data::new(app_state))
                .app_data(auth_state_for_server.clone())
                // OpenAPI documentation endpoints
                .route("/api-docs/openapi.json", web::get().to(routes::api::openapi_json))
                .route("/api-docs", web::get().to(routes::api::scalar_docs))
                // Health endpoints (no auth required)
                .route("/health", web::get().to(routes::api::health_check))
                .route("/ready", web::get().to(routes::api::readiness_check))
                .route("/metrics", web::get().to(routes::api::metrics_handler))
                // Job endpoints
                .route("/api/v1/jobs", web::post().to(routes::api::submit_job))
                .route("/api/v1/jobs", web::get().to(routes::api::list_jobs))
                .route("/api/v1/jobs/{job_id}", web::get().to(routes::api::get_job_status))
                .route("/api/v1/jobs/{job_id}/cancel", web::post().to(routes::api::cancel_job))
                .route("/api/v1/jobs/{job_id}/replay", web::post().to(routes::api::replay_job))
                .route("/api/v1/jobs/{job_id}/retry", web::post().to(routes::api::retry_job))
                .route("/api/v1/jobs/{job_id}/logs", web::get().to(routes::api::get_job_logs))
                .route("/api/v1/jobs/{job_id}/stream", web::get().to(ci_runner::libs::sse::stream_job_updates))
                // Artifact endpoints
                .route("/api/v1/jobs/{job_id}/artifacts", web::post().to(routes::api::upload_artifact))
                .route("/api/v1/jobs/{job_id}/artifacts/{artifact_name}", web::get().to(routes::api::download_artifact))
                .app_data(web::Data::new(event_broadcaster_for_server.clone()))
        })
        .bind(&server_addr)
        .expect("Failed to bind server")
        .run()
        .await
        .expect("Server error");
    });

    // Start background cleanup task
    let workspace_manager_cleanup = Arc::clone(&app.workspace_manager);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(3600)); // Every hour
        loop {
            interval.tick().await;
            if let Err(e) = workspace_manager_cleanup
                .cleanup_old_workspaces(Duration::from_secs(24 * 3600))
                .await
            {
                warn!("Failed to cleanup old workspaces: {}", e);
            }
        }
    });

    // Wait for shutdown signal
    info!("CI Runner started successfully");

    match signal::ctrl_c().await {
        Ok(()) => {
            info!("Received shutdown signal");
        }
        Err(err) => {
            error!("Unable to listen for shutdown signal: {}", err);
        }
    }

    // Graceful shutdown
    info!("Initiating graceful shutdown");
    app.scheduler
        .wait_for_completion(Duration::from_secs(300))
        .await
        .unwrap_or_else(|_| warn!("Graceful shutdown timeout exceeded"));

    server_handle.abort();

    info!("CI Runner stopped");
    Ok(())
}
