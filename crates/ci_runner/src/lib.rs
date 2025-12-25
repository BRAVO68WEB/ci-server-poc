//! CI Runner Library
//!
//! A distributed CI/CD runner system for executing jobs in isolated Docker containers.

// Core modules
pub mod core;
pub mod config;
pub mod models;

// Routes and middleware
pub mod routes;
pub mod middleware;

// Services
pub mod services;

// Storage
pub mod stores;

// Utilities
pub mod utils;

// External library integrations
pub mod libs;

// Re-export commonly used types
pub use config::Config;
pub use models::error::ExecutionError;
pub use models::types::{JobContext, JobEvent, JobResult, JobStatus};

// Re-export for backward compatibility (deprecated, use new paths)
#[deprecated(note = "Use services::executor instead")]
pub use services::executor as executor;
#[deprecated(note = "Use services::scheduler instead")]
pub use services::scheduler as scheduler;
#[deprecated(note = "Use services::cloner instead")]
pub use services::cloner as cloner;
#[deprecated(note = "Use services::parser instead")]
pub use services::parser as parser;
#[deprecated(note = "Use services::event_publisher instead")]
pub use services::event_publisher as event_publisher;
#[deprecated(note = "Use services::log_streamer instead")]
pub use services::log_streamer as log_streamer;
#[deprecated(note = "Use services::artifact_collector instead")]
pub use services::artifact_collector as artifact_collector;
#[deprecated(note = "Use stores::memory instead")]
pub use stores::memory as store;
#[deprecated(note = "Use stores::redis instead")]
pub use stores::redis as redis_store;
#[deprecated(note = "Use stores::adapter instead")]
pub use stores::adapter as store_adapter;
#[deprecated(note = "Use stores::artifacts instead")]
pub use stores::artifacts as artifacts;
#[deprecated(note = "Use models::types instead")]
pub use models::types as types;
#[deprecated(note = "Use models::error instead")]
pub use models::error as error;
#[deprecated(note = "Use middleware::auth instead")]
pub use middleware::auth as auth;
#[deprecated(note = "Use utils::metrics instead")]
pub use utils::metrics as metrics;
#[deprecated(note = "Use utils::workspace instead")]
pub use utils::workspace as workspace;
#[deprecated(note = "Use utils::step_evaluator instead")]
pub use utils::step_evaluator as step_evaluator;
#[deprecated(note = "Use libs::openapi instead")]
pub use libs::openapi as openapi;
#[deprecated(note = "Use libs::scalar instead")]
pub use libs::scalar as scalar;
#[deprecated(note = "Use libs::sse instead")]
pub use libs::sse as sse;
#[deprecated(note = "Use routes::api instead")]
pub use routes::api as api;
