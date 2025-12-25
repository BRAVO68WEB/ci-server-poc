use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ExecutionError {
    #[error("Failed to clone repository: {0}")]
    CloneError(#[from] GitError),

    #[error("Failed to parse runner.yaml: {0}")]
    ParseError(#[from] ParseError),

    #[error("Docker operation failed: {0}")]
    DockerError(#[from] DockerError),

    #[error("Job timed out after {0:?}")]
    Timeout(Duration),

    #[error("System resource exhausted: {0}")]
    ResourceExhausted(String),

    #[error("Authentication failed: {0}")]
    AuthError(String),

    #[error("Network error: {0}")]
    NetworkError(#[from] reqwest::Error),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Configuration error: {0}")]
    ConfigError(String),

    #[error("Queue error: {0}")]
    QueueError(String),
}

#[derive(Debug, Error)]
pub enum GitError {
    #[error("Git command failed: {0}")]
    CommandFailed(String),

    #[error("Repository size exceeds limit")]
    SizeLimitExceeded,

    #[error("Git operation timed out")]
    Timeout,

    #[error("Authentication failed")]
    AuthFailed,

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("Invalid YAML: {0}")]
    InvalidYaml(#[from] serde_yaml::Error),

    #[error("Validation failed: {0}")]
    ValidationFailed(String),

    #[error("File not found: {0}")]
    FileNotFound(String),

    #[error("File too large: {0} bytes")]
    FileTooLarge(usize),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

#[derive(Debug, Error)]
pub enum DockerError {
    #[error("Docker API error: {0}")]
    ApiError(String),

    #[error("Image pull failed: {0}")]
    ImagePullFailed(String),

    #[error("Container creation failed: {0}")]
    ContainerCreationFailed(String),

    #[error("Container execution failed: {0}")]
    ContainerExecutionFailed(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

#[derive(Debug, Error)]
pub enum StreamError {
    #[error("HTTP error: {0}")]
    HttpError(u16),

    #[error("Network error: {0}")]
    NetworkError(#[from] reqwest::Error),

    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),
}

#[derive(Debug, Error)]
pub enum PublishError {
    #[error("HTTP error: {0}")]
    HttpError(u16),

    #[error("Network error: {0}")]
    NetworkError(#[from] reqwest::Error),

    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    #[error("Queue error: {0}")]
    QueueError(String),
}

#[derive(Debug, Error)]
pub enum ScheduleError {
    #[error("Semaphore acquisition failed")]
    SemaphoreAcquisitionFailed,

    #[error("Job already exists: {0}")]
    JobExists(uuid::Uuid),
}
