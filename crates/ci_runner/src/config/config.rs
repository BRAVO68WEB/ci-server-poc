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

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub git_server: GitServerConfig,
    pub executor: ExecutorConfig,
    pub log_streamer: LogStreamerConfig,
    pub security: SecurityConfig,
    pub logging: LoggingConfig,
    pub metrics: MetricsConfig,
    #[serde(default)]
    pub store: StoreConfig,
    #[serde(default)]
    pub auth: AuthConfig,
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

#[derive(Debug, Clone, Deserialize)]
pub struct StoreConfig {
    #[serde(default = "default_store_type")]
    pub store_type: String,
    #[serde(default)]
    pub redis_url: Option<String>,
    #[serde(default = "default_redis_prefix")]
    pub redis_prefix: String,
    #[serde(default = "default_max_history")]
    pub max_history: usize,
}

fn default_store_type() -> String {
    "memory".to_string()
}

fn default_redis_prefix() -> String {
    "ci_runner".to_string()
}

fn default_max_history() -> usize {
    1000
}

impl Default for StoreConfig {
    fn default() -> Self {
        Self {
            store_type: "memory".to_string(),
            redis_url: None,
            redis_prefix: "ci_runner".to_string(),
            max_history: 1000,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub api_keys: Vec<String>,
    #[serde(default)]
    pub api_key_file: Option<PathBuf>,
    #[serde(default = "default_rate_limit")]
    pub rate_limit_per_minute: u32,
}

fn default_rate_limit() -> u32 {
    60
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            api_keys: Vec::new(),
            api_key_file: None,
            rate_limit_per_minute: 60,
        }
    }
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
