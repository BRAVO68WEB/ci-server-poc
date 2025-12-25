use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct JobEvent {
    pub job_id: Uuid,
    pub run_id: Uuid,
    pub repository: RepositoryInfo,
    pub trigger: TriggerInfo,
    pub config_path: String,
    pub timestamp: DateTime<Utc>,
    pub priority: JobPriority,
    pub timeout: Option<Duration>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RepositoryInfo {
    pub owner: String,
    pub name: String,
    pub clone_url: String,
    pub commit_sha: String,
    pub ref_name: String,
    pub ref_type: RefType,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub enum RefType {
    Branch,
    Tag,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TriggerInfo {
    pub event_type: EventType,
    pub actor: String,
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub enum EventType {
    Push,
    Tag,
    PullRequest,
    Manual,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, ToSchema)]
pub enum JobPriority {
    Low = 0,
    Normal = 1,
    High = 2,
    Critical = 3,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RunnerConfig {
    pub image: DockerImage,
    pub on: TriggerConditions,
    pub steps: HashMap<String, Step>,
    #[serde(default)]
    pub global_env: HashMap<String, String>,
    #[serde(default)]
    pub timeout: Option<Duration>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DockerImage {
    pub name: String,
    pub tag: String,
    #[serde(default)]
    pub registry: Option<String>,
    #[serde(default)]
    pub pull_policy: PullPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, ToSchema)]
pub enum PullPolicy {
    Always,
    #[default]
    IfNotPresent,
    Never,
}


#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TriggerConditions {
    #[serde(default)]
    pub push: Vec<String>,
    #[serde(default)]
    pub tag: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Step {
    #[serde(rename = "type")]
    pub step_type: StepType,
    pub scripts: Vec<String>,
    #[serde(default)]
    pub envs: HashMap<String, String>,
    #[serde(default)]
    pub environment: Option<String>,
    #[serde(default)]
    pub timeout: Option<Duration>,
    #[serde(default)]
    pub continue_on_error: bool,
    #[serde(default)]
    #[schema(value_type = Option<String>)]
    pub working_directory: Option<PathBuf>,
    #[serde(default)]
    pub shell: Shell,
    #[serde(default)]
    pub if_condition: Option<String>,
    #[serde(default)]
    pub when: Option<WhenCondition>,
    #[serde(default)]
    pub retry: Option<RetryPolicy>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub enum WhenCondition {
    #[serde(rename = "on_success")]
    OnSuccess,
    #[serde(rename = "on_failure")]
    OnFailure,
    #[serde(rename = "always")]
    Always,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    #[serde(default = "default_backoff_multiplier")]
    pub backoff_multiplier: f64,
    #[serde(default = "default_initial_delay_secs")]
    pub initial_delay_secs: u64,
}

fn default_backoff_multiplier() -> f64 {
    2.0
}

fn default_initial_delay_secs() -> u64 {
    1
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, ToSchema)]
pub enum StepType {
    #[serde(rename = "pre")]
    Pre,
    #[serde(rename = "exec")]
    Exec,
    #[serde(rename = "post")]
    Post,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, ToSchema)]
pub enum Shell {
    #[serde(rename = "bash")]
    #[default]
    Bash,
    #[serde(rename = "sh")]
    Sh,
    #[serde(rename = "python")]
    Python,
    #[serde(rename = "node")]
    Node,
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobCompletionEvent {
    pub job_id: Uuid,
    pub run_id: Uuid,
    pub status: JobStatus,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    pub duration: Duration,
    pub exit_code: i32,
    pub steps: Vec<StepSummary>,
    pub artifacts: Vec<ArtifactInfo>,
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
pub enum JobStatus {
    Success,
    Failed,
    Cancelled,
    TimedOut,
    SystemError,
}

impl JobStatus {
    pub fn routing_suffix(&self) -> &'static str {
        match self {
            JobStatus::Success => "success",
            JobStatus::Failed => "failed",
            JobStatus::Cancelled => "cancelled",
            JobStatus::TimedOut => "timeout",
            JobStatus::SystemError => "error",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct StepSummary {
    pub name: String,
    pub status: JobStatus,
    pub exit_code: i32,
    pub duration: Duration,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ArtifactInfo {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub checksum: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LogEntry {
    pub job_id: Uuid,
    pub run_id: Uuid,
    pub timestamp: DateTime<Utc>,
    pub level: LogLevel,
    pub step_name: Option<String>,
    pub message: String,
    pub sequence: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
pub enum LogLevel {
    Debug,
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone)]
pub struct JobContext {
    pub job_id: Uuid,
    pub run_id: Uuid,
    pub workspace_path: PathBuf,
    pub config: RunnerConfig,
    pub repository: RepositoryInfo,
    pub trigger: TriggerInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct StepResult {
    pub name: String,
    pub step_type: StepType,
    pub exit_code: i32,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct JobResult {
    pub status: JobStatus,
    pub steps: Vec<StepResult>,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
}

impl JobResult {
    pub fn from_steps(steps: Vec<StepResult>) -> Self {
        let started_at = steps.first().map(|s| s.started_at).unwrap_or_else(Utc::now);
        let finished_at = steps.last().map(|s| s.finished_at).unwrap_or_else(Utc::now);

        let status = if steps.iter().any(|s| s.exit_code != 0) {
            JobStatus::Failed
        } else {
            JobStatus::Success
        };

        Self {
            status,
            steps,
            started_at,
            finished_at,
        }
    }

    pub fn duration(&self) -> Duration {
        self.finished_at
            .signed_duration_since(self.started_at)
            .to_std()
            .unwrap_or_default()
    }
}
