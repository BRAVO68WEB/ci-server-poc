use crate::models::error::{ExecutionError, ParseError};
use crate::models::types::{RunnerConfig, Step};
use indexmap::IndexMap;
use regex::Regex;
use std::path::Path;
use std::time::Duration;
use tokio::fs;
use tracing::instrument;

pub struct TaskParser {
    max_file_size: usize,
}

impl TaskParser {
    pub fn new(max_file_size: usize) -> Self {
        Self { max_file_size }
    }

    #[instrument(skip(self), fields(workspace = %workspace.display()))]
    pub async fn parse(&self, workspace: &Path) -> Result<RunnerConfig, ExecutionError> {
        let config_path = workspace.join("runner.yaml");

        // Read file with size limit
        let content = self.read_with_limit(&config_path).await?;

        // Parse YAML
        let config: RunnerConfig =
            serde_yaml::from_str(&content).map_err(ParseError::InvalidYaml)?;

        // Validate
        self.validate(&config)?;

        Ok(config)
    }

    async fn read_with_limit(&self, path: &Path) -> Result<String, ExecutionError> {
        let metadata = fs::metadata(path).await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                ParseError::FileNotFound(path.display().to_string())
            } else {
                ParseError::IoError(e)
            }
        })?;

        if metadata.len() > self.max_file_size as u64 {
            return Err(ExecutionError::ParseError(ParseError::FileTooLarge(
                metadata.len() as usize,
            )));
        }

        let content = fs::read_to_string(path)
            .await
            .map_err(ParseError::IoError)?;

        Ok(content)
    }

    fn validate(&self, config: &RunnerConfig) -> Result<(), ExecutionError> {
        // Validate image
        self.validate_image(&config.image)?;

        // Validate triggers
        self.validate_triggers(&config.on)?;

        // Validate steps
        self.validate_steps(&config.steps)?;

        Ok(())
    }

    fn validate_image(&self, image: &crate::models::types::DockerImage) -> Result<(), ExecutionError> {
        if image.name.is_empty() {
            return Err(ExecutionError::ParseError(ParseError::ValidationFailed(
                "Image name cannot be empty".to_string(),
            )));
        }

        if image.tag.is_empty() {
            return Err(ExecutionError::ParseError(ParseError::ValidationFailed(
                "Image tag cannot be empty".to_string(),
            )));
        }

        if let Some(ref registry) = image.registry {
            // Basic hostname validation
            if registry.is_empty() {
                return Err(ExecutionError::ParseError(ParseError::ValidationFailed(
                    "Registry cannot be empty if specified".to_string(),
                )));
            }
        }

        Ok(())
    }

    fn validate_triggers(
        &self,
        triggers: &crate::models::types::TriggerConditions,
    ) -> Result<(), ExecutionError> {
        if triggers.push.is_empty() && triggers.tag.is_empty() {
            return Err(ExecutionError::ParseError(ParseError::ValidationFailed(
                "At least one trigger condition (push or tag) must be specified".to_string(),
            )));
        }

        // Validate patterns (basic check)
        for pattern in &triggers.push {
            if pattern.is_empty() {
                return Err(ExecutionError::ParseError(ParseError::ValidationFailed(
                    "Push pattern cannot be empty".to_string(),
                )));
            }
        }

        for pattern in &triggers.tag {
            if pattern.is_empty() {
                return Err(ExecutionError::ParseError(ParseError::ValidationFailed(
                    "Tag pattern cannot be empty".to_string(),
                )));
            }
        }

        Ok(())
    }

    fn validate_steps(&self, steps: &IndexMap<String, Step>) -> Result<(), ExecutionError> {
        if steps.is_empty() {
            return Err(ExecutionError::ParseError(ParseError::ValidationFailed(
                "At least one step must be defined".to_string(),
            )));
        }

        if steps.len() > 50 {
            return Err(ExecutionError::ParseError(ParseError::ValidationFailed(
                format!("Too many steps: {} (max 50)", steps.len()),
            )));
        }

        let env_key_regex = Regex::new(r"^[A-Z_][A-Z0-9_]*$").map_err(|e| {
            ExecutionError::ParseError(ParseError::ValidationFailed(format!(
                "Regex compilation failed: {}",
                e
            )))
        })?;

        for (name, step) in steps {
            // Validate step name
            if name.is_empty() {
                return Err(ExecutionError::ParseError(ParseError::ValidationFailed(
                    "Step name cannot be empty".to_string(),
                )));
            }

            // Validate scripts
            if step.scripts.is_empty() {
                return Err(ExecutionError::ParseError(ParseError::ValidationFailed(
                    format!("Step '{}' must have at least one script", name),
                )));
            }

            // Check script size
            let total_size: usize = step.scripts.iter().map(|s| s.len()).sum();

            if total_size > 1024 * 1024 {
                // 1MB
                return Err(ExecutionError::ParseError(ParseError::ValidationFailed(
                    format!("Step '{}' scripts exceed 1MB limit", name),
                )));
            }

            // Validate environment variables
            for key in step.envs.keys() {
                if !env_key_regex.is_match(key) {
                    return Err(ExecutionError::ParseError(ParseError::ValidationFailed(
                        format!(
                            "Invalid environment variable name '{}' in step '{}'",
                            key, name
                        ),
                    )));
                }
            }

            // Validate timeout
            if let Some(timeout) = step.timeout {
                let min_timeout = Duration::from_secs(1);
                let max_timeout = Duration::from_secs(24 * 3600); // 24 hours

                if timeout < min_timeout || timeout > max_timeout {
                    return Err(ExecutionError::ParseError(ParseError::ValidationFailed(
                        format!("Step '{}' timeout must be between 1s and 24h", name),
                    )));
                }
            }
        }

        Ok(())
    }
}
