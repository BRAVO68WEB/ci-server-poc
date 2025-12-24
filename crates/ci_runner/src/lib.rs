//! CI Runner Library
//!
//! A distributed CI/CD runner system for executing jobs in isolated Docker containers.

pub mod api;
pub mod cloner;
pub mod config;
pub mod error;
pub mod event_publisher;
pub mod executor;
pub mod log_streamer;
pub mod parser;
pub mod queue;
pub mod scheduler;
pub mod types;
pub mod workspace;

// Re-export commonly used types
pub use config::Config;
pub use error::ExecutionError;
pub use types::{JobContext, JobEvent, JobResult, JobStatus};
