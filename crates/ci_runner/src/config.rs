use serde::{Deserialize, Deserializer};
use std::path::PathBuf;
use std::time::Duration;

// Custom deserializer for Duration that accepts integer seconds
fn deserialize_duration<'de, D>(deserializer: D) -> Result<Duration, D::Error>
where
    D: Deserializer<'de>,
{
    
    let secs = u64::deserialize(deserializer)?;
    Ok(Duration::from_secs(secs))
}

// Custom deserializer for Option<Duration>
fn deserialize_option_duration<'de, D>(deserializer: D) -> Result<Option<Duration>, D::Error>
where
    D: Deserializer<'de>,
{
    
    match Option::<u64>::deserialize(deserializer)? {
        Some(secs) => Ok(Some(Duration::from_secs(secs))),
        None => Ok(None),
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub git_server: GitServerConfig,
    pub queue: QueueConfig,
    pub executor: ExecutorConfig,
    pub log_streamer: LogStreamerConfig,
    pub security: SecurityConfig,
    pub logging: LoggingConfig,
    pub metrics: MetricsConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub metrics_port: u16,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GitServerConfig {
    pub base_url: String,
    pub service_token_path: PathBuf,
    #[serde(deserialize_with = "deserialize_duration")]
    pub api_timeout: Duration,
}

#[derive(Debug, Clone, Deserialize)]
pub struct QueueConfig {
    pub host: String,
    pub port: u16,
    pub virtual_host: String,
    pub queue_name: String,
    pub dead_letter_queue: String,
    pub prefetch_count: u16,
    pub username_file: Option<PathBuf>,
    pub password_file: Option<PathBuf>,
    #[serde(deserialize_with = "deserialize_option_duration")]
    pub connection_timeout: Option<Duration>,
    #[serde(deserialize_with = "deserialize_option_duration")]
    pub heartbeat: Option<Duration>,
    pub tls: Option<TlsConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TlsConfig {
    pub enabled: bool,
    pub ca_cert_path: Option<PathBuf>,
    pub client_cert_path: Option<PathBuf>,
    pub client_key_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExecutorConfig {
    pub max_concurrent_jobs: usize,
    pub workspace_root: PathBuf,
    pub cache_root: PathBuf,
    pub docker: DockerConfig,
    pub resources: ResourceConfig,
    pub timeouts: TimeoutConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DockerConfig {
    pub socket: String,
    pub registry_auth: Option<PathBuf>,
    pub network: Option<String>,
    pub network_mode: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ResourceConfig {
    pub cpu_limit: f64,
    pub memory_limit: String,
    pub storage_limit: String,
    pub pids_limit: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TimeoutConfig {
    #[serde(deserialize_with = "deserialize_duration")]
    pub default: Duration,
    #[serde(deserialize_with = "deserialize_duration")]
    pub max: Duration,
    #[serde(deserialize_with = "deserialize_duration")]
    pub git_clone: Duration,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LogStreamerConfig {
    pub git_server: GitServerLogConfig,
    pub buffer: BufferConfig,
    pub rate_limit: RateLimitConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GitServerLogConfig {
    pub base_url: String,
    pub endpoint: String,
    pub auth_token_env: String,
    #[serde(deserialize_with = "deserialize_duration")]
    pub timeout: Duration,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BufferConfig {
    pub size: usize,
    #[serde(deserialize_with = "deserialize_duration")]
    pub flush_interval: Duration,
    pub max_retries: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RateLimitConfig {
    pub max_bytes_per_second: String,
    pub burst_size: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SecurityConfig {
    pub secret_backend: String,
    pub vault_addr: Option<String>,
    pub vault_token_path: Option<PathBuf>,
    pub seccomp_profile: Option<PathBuf>,
    pub apparmor_profile: Option<String>,
    pub network: NetworkSecurityConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NetworkSecurityConfig {
    pub isolation: String,
    pub block_metadata: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LoggingConfig {
    pub level: String,
    pub format: String,
    pub output: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MetricsConfig {
    pub enabled: bool,
    pub port: u16,
    pub path: String,
}

impl Config {
    pub fn load(path: Option<&str>) -> Result<Self, config::ConfigError> {
        let config_path = path.unwrap_or("/etc/ci-runner/config.yaml");
        let settings = config::Config::builder()
            .add_source(config::File::with_name(config_path))
            .add_source(config::Environment::with_prefix("CI"))
            .build()?;

        settings.try_deserialize()
    }
}
