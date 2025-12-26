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
