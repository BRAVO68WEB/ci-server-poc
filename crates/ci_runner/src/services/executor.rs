use crate::models::error::{DockerError, ExecutionError};
use crate::models::types::{JobContext, JobResult, JobStatus, Step, StepResult, StepType};
use bollard::Docker;
use bollard::exec::{CreateExecOptions, StartExecOptions};
use bollard::models::{ContainerCreateBody, HostConfig};
use bollard::query_parameters::{
    CreateContainerOptions, RemoveContainerOptions, StartContainerOptions, StopContainerOptions,
};
use chrono::Utc;
use futures_util::StreamExt;
use std::time::Duration;
use tracing::{info, instrument, warn};

pub struct JobExecutor {
    docker: Docker,
    config: ExecutorConfig,
    log_streamer: Option<std::sync::Arc<dyn LogStreamerTrait>>,
}

pub trait LogStreamerTrait: Send + Sync {
    fn send(&self, job_id: uuid::Uuid, run_id: uuid::Uuid, step_name: Option<String>, level: crate::models::types::LogLevel, message: &[u8]);
}

#[derive(Debug, Clone)]
pub struct ExecutorConfig {
    pub cpu_limit: f64,
    pub memory_limit: u64,
    pub pids_limit: i64,
    pub network_mode: String,
    pub default_timeout: Duration,
    pub max_timeout: Duration,
}

impl JobExecutor {
    pub fn new(docker: Docker, config: ExecutorConfig) -> Self {
        Self {
            docker,
            config,
            log_streamer: None,
        }
    }

    pub fn with_log_streamer(mut self, streamer: std::sync::Arc<dyn LogStreamerTrait>) -> Self {
        self.log_streamer = Some(streamer);
        self
    }

    #[instrument(skip(self), fields(job_id = %job.job_id))]
    pub async fn execute(&self, job: JobContext) -> Result<JobResult, ExecutionError> {
        info!("Starting job execution");
        let start_time = Utc::now();

        // Pull Docker image
        self.pull_image(&job.config.image).await?;

        // Create container
        let container_id = self.create_container(&job).await?;

        // Start container - handle mount errors (Docker Desktop Mac limitation)
        if let Err(e) = self
            .docker
            .start_container(&container_id, None::<StartContainerOptions>)
            .await
        {
            let error_msg = e.to_string();
            if error_msg.contains("mounts denied")
                || error_msg.contains("not shared")
                || error_msg.contains("not known to Docker")
            {
                // Docker Desktop on Mac doesn't allow mounting from inside container to container
                // Clean up the container and return a clearer error
                let _ = self
                    .docker
                    .remove_container(&container_id, None::<RemoveContainerOptions>)
                    .await;
                return Err(ExecutionError::DockerError(
                    DockerError::ContainerCreationFailed(format!(
                        "Docker mount failed (Docker Desktop Mac limitation). Workspace path {} cannot be mounted from inside container. Consider using a Docker volume instead of bind mount.",
                        job.workspace_path.display()
                    )),
                ));
            }
            return Err(ExecutionError::DockerError(DockerError::ApiError(format!(
                "Failed to start container: {}",
                e
            ))));
        }

        // Ensure cleanup
        let cleanup_container = |id: &str| {
            let docker = self.docker.clone();
            let id = id.to_string();
            async move {
                if let Err(e) = docker
                    .stop_container(&id, None::<StopContainerOptions>)
                    .await
                {
                    warn!("Failed to stop container {}: {}", id, e);
                }
                if let Err(e) = docker
                    .remove_container(&id, None::<RemoveContainerOptions>)
                    .await
                {
                    warn!("Failed to remove container {}: {}", id, e);
                }
            }
        };

        // Execute steps in order (pre -> exec -> post)
        let mut results = Vec::new();
        let mut job_failed = false;
        
        // Create evaluation context
        let mut eval_context = crate::utils::step_evaluator::StepEvaluationContext::from_job_context(&job);

        for step_type in [StepType::Pre, StepType::Exec, StepType::Post] {
            if job_failed && step_type == StepType::Post {
                // Still execute post steps even if pre/exec failed
            } else if job_failed {
                continue;
            }

            let steps = self.get_steps_by_type(&job.config.steps, step_type);

            for (name, step) in steps {
                // Evaluate if condition
                if let Some(ref if_condition) = step.if_condition {
                    match crate::utils::step_evaluator::StepEvaluator::evaluate_if_condition(
                        if_condition,
                        &eval_context,
                    ) {
                        Ok(should_run) => {
                            if !should_run {
                                info!(step = %name, condition = %if_condition, "Skipping step due to if condition");
                                continue;
                            }
                        }
                        Err(e) => {
                            warn!(step = %name, condition = %if_condition, error = %e, "Failed to evaluate if condition, skipping step");
                            continue;
                        }
                    }
                }
                
                // Evaluate when condition
                if let Some(ref when) = step.when {
                    if !crate::utils::step_evaluator::StepEvaluator::should_run_when(when, job_failed) {
                        info!(step = %name, "Skipping step due to when condition");
                        continue;
                    }
                }
                
                // Execute step with retry logic
                let mut result = None;
                let max_attempts = step.retry.as_ref().map(|r| r.max_attempts).unwrap_or(1);
                
                for attempt in 1..=max_attempts {
                    if attempt > 1 {
                        if let Some(ref retry_policy) = step.retry {
                            let delay = crate::utils::step_evaluator::StepEvaluator::calculate_retry_delay(
                                retry_policy,
                                attempt,
                            );
                            info!(step = %name, attempt = attempt, delay_secs = delay.as_secs(), "Retrying step after delay");
                            tokio::time::sleep(delay).await;
                        }
                    }
                    
                    match self.execute_step(&container_id, &name, &step, &job).await {
                        Ok(step_result) => {
                            result = Some(step_result.clone());
                            
                            // If step succeeded or we're on last attempt, break
                            if step_result.exit_code == 0 || attempt == max_attempts {
                                break;
                            }
                            
                            // If continue_on_error is true, break even on failure
                            if step.continue_on_error {
                                break;
                            }
                        }
                        Err(e) => {
                            if attempt == max_attempts {
                                return Err(e);
                            }
                            warn!(step = %name, attempt = attempt, error = %e, "Step execution failed, will retry");
                        }
                    }
                }
                
                let step_result = result.expect("Step result should be set");
                results.push(step_result.clone());
                
                // Update evaluation context with this step's result
                eval_context.previous_steps.insert(name.clone(), step_result.clone());

                if step_result.exit_code != 0 && !step.continue_on_error {
                    job_failed = true;
                    if step_type != StepType::Post {
                        break;
                    }
                }
            }
        }

        // Cleanup container
        cleanup_container(&container_id).await;

        // Aggregate results
        let finished_at = Utc::now();
        let status = if results.iter().any(|r| r.exit_code != 0) {
            JobStatus::Failed
        } else {
            JobStatus::Success
        };

        let result = JobResult {
            status,
            steps: results,
            started_at: start_time,
            finished_at,
        };

        info!(
            duration_ms = result.duration().as_millis(),
            status = ?result.status,
            "Job execution completed"
        );

        Ok(result)
    }

    async fn pull_image(&self, image: &crate::models::types::DockerImage) -> Result<(), ExecutionError> {
        let image_name = if let Some(ref registry) = image.registry {
            format!("{}/{}:{}", registry, image.name, image.tag)
        } else {
            format!("{}:{}", image.name, image.tag)
        };

        match image.pull_policy {
            crate::models::types::PullPolicy::Never => {
                info!("Skipping image pull (policy: Never)");
                return Ok(());
            }
            crate::models::types::PullPolicy::IfNotPresent => {
                // Check if image exists
                if self.docker.inspect_image(&image_name).await.is_ok() {
                    info!("Image {} already exists, skipping pull", image_name);
                    return Ok(());
                }
            }
            crate::models::types::PullPolicy::Always => {
                // Always pull
            }
        }

        info!("Pulling image: {}", image_name);

        use bollard::query_parameters::CreateImageOptions;
        let options = Some(CreateImageOptions {
            from_image: Some(image_name.clone()),
            ..Default::default()
        });

        let mut stream = self.docker.create_image(options, None, None);
        while let Some(msg) = stream.next().await {
            match msg {
                Ok(_) => {
                    // Image pull progress
                }
                Err(e) => {
                    return Err(ExecutionError::DockerError(DockerError::ImagePullFailed(
                        format!("Failed to pull image {}: {}", image_name, e),
                    )));
                }
            }
        }

        Ok(())
    }

    async fn create_container(&self, job: &JobContext) -> Result<String, ExecutionError> {
        let image_name = if let Some(ref registry) = job.config.image.registry {
            format!(
                "{}/{}:{}",
                registry, job.config.image.name, job.config.image.tag
            )
        } else {
            format!("{}:{}", job.config.image.name, job.config.image.tag)
        };

        let container_name = format!("ci-job-{}", job.job_id);

        // Clean up any existing container with the same name (from previous failed attempts)
        if let Ok(existing) = self
            .docker
            .inspect_container(
                &container_name,
                None::<bollard::query_parameters::InspectContainerOptions>,
            )
            .await {
            if let Some(id) = existing.id.as_ref() {
                warn!("Found existing container {}, removing it", container_name);
                let _ = self
                    .docker
                    .stop_container(id, None::<StopContainerOptions>)
                    .await;
                let _ = self
                    .docker
                    .remove_container(id, None::<RemoveContainerOptions>)
                    .await;
            }
        }

        let mut host_config = HostConfig::default();
        host_config.cpu_quota = Some((self.config.cpu_limit * 100_000.0) as i64);
        host_config.cpu_period = Some(100_000);
        host_config.memory = Some(self.config.memory_limit as i64);
        host_config.memory_swap = Some(self.config.memory_limit as i64); // No swap
        host_config.pids_limit = Some(self.config.pids_limit);

        // Mount the Docker volume instead of bind mount (required for Docker Desktop Mac)
        // The workspace volume is mounted at /app/workspaces in ci-runner-dev container
        // We mount the entire volume and use the job_id subdirectory
        host_config.binds = Some(vec![format!("ci-workspaces:/workspace:rw")]);
        host_config.network_mode = Some(self.config.network_mode.clone());

        let mut env_vars = Vec::new();
        env_vars.push(format!("CI_JOB_ID={}", job.job_id));
        env_vars.push(format!("CI_RUN_ID={}", job.run_id));
        env_vars.push(format!("CI_REPO_OWNER={}", job.repository.owner));
        env_vars.push(format!("CI_REPO_NAME={}", job.repository.name));
        env_vars.push(format!("CI_COMMIT_SHA={}", job.repository.commit_sha));
        env_vars.push(format!("CI_REF_NAME={}", job.repository.ref_name));

        // Add global env vars
        for (key, value) in &job.config.global_env {
            env_vars.push(format!("{}={}", key, value));
        }

        // Set working directory to the job-specific subdirectory within the volume
        let job_workspace_dir = format!("/workspace/{}", job.job_id);

        // Use a command that keeps the container running so we can exec into it
        // Alpine and most base images exit immediately without a command
        let keep_alive_cmd = vec![
            "sh".to_string(),
            "-c".to_string(),
            "trap 'exit 0' TERM; while :; do sleep 1; done".to_string(),
        ];

        let config = ContainerCreateBody {
            image: Some(image_name),
            cmd: Some(keep_alive_cmd),
            working_dir: Some(job_workspace_dir),
            env: Some(env_vars),
            host_config: Some(host_config),
            attach_stdout: Some(true),
            attach_stderr: Some(true),
            tty: Some(false),
            ..Default::default()
        };

        let options = Some(CreateContainerOptions {
            name: Some(container_name.clone()),
            platform: "linux".to_string(),
        });

        // Try to create container, handle name conflicts
        let result = match self
            .docker
            .create_container(options.clone(), config.clone())
            .await
        {
            Ok(r) => r,
            Err(e) => {
                let error_str = e.to_string();
                if error_str.contains("already in use") || error_str.contains("Conflict") {
                    // Try cleanup and retry once
                    warn!("Container name conflict detected, attempting cleanup and retry");
                    if let Ok(existing) = self
                        .docker
                        .inspect_container(
                            &container_name,
                            None::<bollard::query_parameters::InspectContainerOptions>,
                        )
                        .await
                    {
                        if let Some(id) = existing.id.as_ref() {
                            let _ = self
                                .docker
                                .stop_container(id, None::<StopContainerOptions>)
                                .await;
                            let _ = self
                                .docker
                                .remove_container(id, None::<RemoveContainerOptions>)
                                .await;
                            // Retry creation
                            self.docker
                                .create_container(options, config)
                                .await
                                .map_err(|e| {
                                    ExecutionError::DockerError(
                                        DockerError::ContainerCreationFailed(format!(
                                            "Failed to create container after cleanup: {}",
                                            e
                                        )),
                                    )
                                })?
                        } else {
                            return Err(ExecutionError::DockerError(
                                DockerError::ContainerCreationFailed(format!(
                                    "Failed to create container: {}",
                                    e
                                )),
                            ));
                        }
                    } else {
                        return Err(ExecutionError::DockerError(
                            DockerError::ContainerCreationFailed(format!(
                                "Failed to create container: {}",
                                e
                            )),
                        ));
                    }
                } else {
                    return Err(ExecutionError::DockerError(
                        DockerError::ContainerCreationFailed(format!(
                            "Failed to create container: {}",
                            e
                        )),
                    ));
                }
            }
        };

        Ok(result.id)
    }

    fn get_steps_by_type(
        &self,
        steps: &std::collections::HashMap<String, Step>,
        step_type: StepType,
    ) -> Vec<(String, Step)> {
        steps
            .iter()
            .filter(|(_, step)| step.step_type == step_type)
            .map(|(name, step)| (name.clone(), step.clone()))
            .collect()
    }

    async fn execute_step(
        &self,
        container_id: &str,
        name: &str,
        step: &Step,
        job: &JobContext,
    ) -> Result<StepResult, ExecutionError> {
        let start_time = Utc::now();

        // Prepare execution command
        let script = self.prepare_script(step)?;
        let exec_config = self.create_exec_config(step, &script, job)?;

        // Create exec instance
        let exec = self
            .docker
            .create_exec(container_id, exec_config)
            .await
            .map_err(|e| {
                ExecutionError::DockerError(DockerError::ContainerExecutionFailed(format!(
                    "Failed to create exec: {}",
                    e
                )))
            })?;

        // Start execution with streaming
        use bollard::exec::StartExecResults;
        let exec_result: StartExecResults = self
            .docker
            .start_exec(
                &exec.id,
                Some(StartExecOptions {
                    detach: false,
                    tty: false,
                    output_capacity: Some(1024 * 1024), // 1MB buffer
                }),
            )
            .await
            .map_err(|e| {
                ExecutionError::DockerError(DockerError::ContainerExecutionFailed(format!(
                    "Failed to start exec: {}",
                    e
                )))
            })?;

        // Stream logs
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        match exec_result {
            StartExecResults::Attached { mut output, .. } => {
                while let Some(msg) = output.next().await {
                    match msg {
                        Ok(bollard::container::LogOutput::StdOut { message }) => {
                            stdout.extend_from_slice(&message);
                            if let Some(ref streamer) = self.log_streamer {
                                streamer.send(job.job_id, job.run_id, Some(name.to_string()), crate::models::types::LogLevel::Info, &message);
                            }
                        }
                        Ok(bollard::container::LogOutput::StdErr { message }) => {
                            stderr.extend_from_slice(&message);
                            if let Some(ref streamer) = self.log_streamer {
                                streamer.send(job.job_id, job.run_id, Some(name.to_string()), crate::models::types::LogLevel::Error, &message);
                            }
                        }
                        Ok(bollard::container::LogOutput::StdIn { message }) => {
                            // Ignore stdin messages
                            let _ = message;
                        }
                        Ok(bollard::container::LogOutput::Console { message }) => {
                            stdout.extend_from_slice(&message);
                        }
                        Err(e) => {
                            warn!("Error reading exec output: {}", e);
                            break;
                        }
                    }
                }
            }
            StartExecResults::Detached => {
                return Err(ExecutionError::DockerError(
                    DockerError::ContainerExecutionFailed(
                        "Exec started in detached mode".to_string(),
                    ),
                ));
            }
        }

        // Get exit code
        let inspect = self.docker.inspect_exec(&exec.id).await.map_err(|e| {
            ExecutionError::DockerError(DockerError::ContainerExecutionFailed(format!(
                "Failed to inspect exec: {}",
                e
            )))
        })?;

        let exit_code = inspect.exit_code.unwrap_or(-1) as i32;

        Ok(StepResult {
            name: name.to_string(),
            step_type: step.step_type,
            exit_code,
            started_at: start_time,
            finished_at: Utc::now(),
            stdout: String::from_utf8_lossy(&stdout).to_string(),
            stderr: String::from_utf8_lossy(&stderr).to_string(),
        })
    }

    fn prepare_script(&self, step: &Step) -> Result<String, ExecutionError> {
        let script = step.scripts.join("\n");
        Ok(format!("set -e\n{}", script))
    }

    fn create_exec_config(
        &self,
        step: &Step,
        script: &str,
        _job: &JobContext,
    ) -> Result<CreateExecOptions<String>, ExecutionError> {
        let shell_cmd = match step.shell {
            crate::models::types::Shell::Bash => "bash",
            crate::models::types::Shell::Sh => "sh",
            crate::models::types::Shell::Python => "python",
            crate::models::types::Shell::Node => "node",
        };

        let mut cmd = vec![shell_cmd.to_string(), "-c".to_string(), script.to_string()];

        if let Some(ref working_dir) = step.working_directory {
            // Note: Docker exec doesn't support changing working directory directly
            // We'd need to wrap it in a shell command
            cmd = vec![
                shell_cmd.to_string(),
                "-c".to_string(),
                format!("cd {} && {}", working_dir.display(), script),
            ];
        }

        let mut env_vars = Vec::new();

        // Add step-specific env vars
        for (key, value) in &step.envs {
            env_vars.push(format!("{}={}", key, value));
        }

        Ok(CreateExecOptions {
            attach_stdout: Some(true),
            attach_stderr: Some(true),
            attach_stdin: Some(false),
            cmd: Some(cmd),
            env: Some(env_vars),
            working_dir: step
                .working_directory
                .as_ref()
                .map(|p| p.to_string_lossy().to_string()),
            privileged: Some(false),
            user: None,
            detach_keys: None,
            tty: Some(false),
        })
    }
}
