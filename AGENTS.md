# CI Runner - Agent Context Documentation

## Project Overview

**CI Runner** is a distributed CI/CD runner system built with Rust, designed to execute CI jobs in isolated Docker containers. It provides a scalable, secure, and flexible platform for running continuous integration and deployment pipelines.

### Key Features

- **HTTP-based Job Submission**: RESTful API for job submission and management (migrated from RabbitMQ)
- **Docker Isolation**: Each job runs in an isolated Docker container with configurable resource limits
- **Git Integration**: Secure repository cloning with authentication (Bearer tokens, SSH keys, Basic Auth)
- **Real-time Logging**: Stream logs to external systems (Git Server) in real-time
- **Job Scheduling**: Concurrent job execution with configurable limits
- **Artifact Management**: Collect, compress, and store build artifacts (local filesystem or S3)
- **Job Replay/Retry**: Re-run jobs with the same or modified configuration
- **Server-Sent Events (SSE)**: Real-time job status updates
- **Conditional Steps**: Execute steps based on conditions (`if`, `when`, `retry`)
- **OpenAPI Documentation**: Interactive API documentation with Scalar UI
- **Prometheus Metrics**: Expose metrics for monitoring and alerting
- **OpenTelemetry Support**: Distributed tracing capabilities
- **Multiple Storage Backends**: In-memory or Redis for job state persistence
- **S3 Artifact Storage**: Store artifacts in S3-compatible storage

## Architecture

### High-Level Architecture

```
┌─────────────┐
│   Client    │
│  (HTTP API) │
└──────┬──────┘
       │
       │ POST /api/v1/jobs
       ▼
┌─────────────────────────────────────┐
│         CI Runner Server            │
│  ┌───────────────────────────────┐ │
│  │   HTTP API (Actix-web)        │ │
│  │   - Job submission            │ │
│  │   - Job status/streaming       │ │
│  │   - Artifact management        │ │
│  └───────────┬───────────────────┘ │
│              │                     │
│  ┌───────────▼───────────────────┐ │
│  │   Job Scheduler               │ │
│  │   - Concurrent execution      │ │
│  │   - Resource management       │ │
│  └───────────┬───────────────────┘ │
│              │                     │
│  ┌───────────▼───────────────────┐ │
│  │   Job Executor                │ │
│  │   - Docker container mgmt     │ │
│  │   - Step execution            │ │
│  │   - Log streaming             │ │
│  └───────────┬───────────────────┘ │
│              │                     │
│  ┌───────────▼───────────────────┐ │
│  │   Repository Cloner           │ │
│  │   - Git clone                 │ │
│  │   - Authentication            │ │
│  └───────────────────────────────┘ │
└─────────────────────────────────────┘
       │
       │ Docker API
       ▼
┌─────────────────────────────────────┐
│      Docker Containers              │
│  ┌───────────────────────────────┐ │
│  │   Job Container               │ │
│  │   - Isolated execution        │ │
│  │   - Volume mounts             │ │
│  │   - Resource limits           │ │
│  └───────────────────────────────┘ │
└─────────────────────────────────────┘
```

### Component Interaction Flow

1. **Job Submission**: Client sends HTTP POST request to `/api/v1/jobs` with job configuration
2. **Authentication**: API key authentication (if enabled) validates the request
3. **Job Scheduling**: Scheduler queues the job and manages concurrent execution
4. **Repository Cloning**: Cloner fetches the repository code into workspace
5. **Configuration Parsing**: Parser reads `.stasis-ci.yaml` from cloned repository
6. **Job Execution**: Executor creates Docker container and runs steps sequentially
7. **Log Streaming**: Logs are streamed in real-time to external systems
8. **Artifact Collection**: Post-steps collect and compress artifacts
9. **Artifact Upload**: Artifacts are uploaded to storage (local or S3)
10. **Job Completion**: Results are stored, events published, workspace cleaned up

## Codebase Structure

### Directory Organization

```
crates/ci_runner/src/
├── main.rs                 # Application entry point
├── lib.rs                  # Library root, module declarations
│
├── config/                 # Configuration management
│   ├── config.rs          # Configuration structs and loading
│   └── mod.rs
│
├── core/                   # Core application logic
│   ├── app.rs             # Application initialization
│   └── mod.rs
│
├── models/                 # Data models and types
│   ├── types.rs           # Core data structures (Job, Step, etc.)
│   ├── error.rs           # Error types and handling
│   └── mod.rs
│
├── services/               # Business logic services
│   ├── scheduler.rs       # Job scheduling and concurrency
│   ├── executor.rs        # Docker container execution
│   ├── cloner.rs         # Git repository cloning
│   ├── parser.rs         # .stasis-ci.yaml parsing
│   ├── log_streamer.rs    # Log streaming to external systems
│   ├── event_publisher.rs # Job completion events
│   ├── artifact_collector.rs  # Artifact discovery
│   ├── artifact_compressor.rs # Artifact compression (tar.gz, zip)
│   └── mod.rs
│
├── stores/                 # Data persistence
│   ├── memory.rs         # In-memory job store
│   ├── redis.rs          # Redis-backed job store
│   ├── artifacts.rs      # Local artifact storage
│   ├── s3_artifacts.rs   # S3 artifact storage
│   ├── artifact_trait.rs # Artifact storage trait
│   ├── adapter.rs        # Store adapter for abstraction
│   └── mod.rs
│
├── routes/                 # HTTP API routes
│   ├── api.rs            # All API endpoints
│   └── mod.rs
│
├── middleware/             # HTTP middleware
│   ├── auth.rs           # API key authentication
│   └── mod.rs
│
├── libs/                   # Shared libraries
│   ├── sse.rs            # Server-Sent Events implementation
│   ├── openapi.rs        # OpenAPI specification generation
│   ├── scalar.rs         # Scalar UI HTML
│   └── mod.rs
│
└── utils/                  # Utility functions
    ├── metrics.rs        # Prometheus metrics
    ├── step_evaluator.rs # Step condition evaluation
    ├── workspace.rs      # Workspace management
    └── mod.rs
```

## Key Components

### 1. Job Scheduler (`services/scheduler.rs`)

**Responsibility**: Manages job queue and concurrent execution limits.

**Key Features**:
- Configurable maximum concurrent jobs
- Job queuing and prioritization
- Resource tracking
- Graceful shutdown handling

**Key Types**:
- `JobScheduler`: Main scheduler struct
- `JobEvent`: Job submission event
- `JobContext`: Runtime job context

### 2. Job Executor (`services/executor.rs`)

**Responsibility**: Executes jobs in Docker containers.

**Key Features**:
- Docker container lifecycle management
- Step execution (pre, exec, post)
- Working directory management
- Environment variable injection
- Timeout handling
- Resource limits (CPU, memory, PIDs)
- Volume mounting (Docker volumes for workspace sharing)

**Key Types**:
- `JobExecutor`: Main executor struct
- `ExecutorConfig`: Executor configuration
- `StepResult`: Step execution result
- `JobResult`: Complete job result

**Important Notes**:
- Uses Docker volumes (`ci-workspaces`) for workspace sharing between ci-runner-dev container and job containers
- Workspace path: `/app/workspaces/{job_id}` in ci-runner-dev → `/workspace/{job_id}` in job containers
- Currently investigating volume mount issues where files cloned to host workspace aren't visible in job containers

### 3. Repository Cloner (`services/cloner.rs`)

**Responsibility**: Clones Git repositories securely.

**Key Features**:
- Multiple authentication methods (Bearer token, SSH key, Basic Auth)
- Shallow cloning support
- Commit checkout and validation
- Submodule initialization
- Git LFS support
- Timeout handling

**Key Types**:
- `RepositoryCloner`: Main cloner struct
- `ServiceAuth`: Authentication credentials
- `TokenType`: Authentication type enum

**Cloning Process**:
1. Clone to temporary directory
2. Move contents to workspace directory
3. Checkout specific commit
4. Initialize submodules (if present)
5. Pull LFS objects (if enabled)
6. Validate commit SHA

### 4. Configuration Parser (`services/parser.rs`)

**Responsibility**: Parses `.stasis-ci.yaml` configuration files.

**Key Features**:
- YAML parsing with validation
- Step type detection (pre, exec, post)
- Conditional step evaluation
- Retry policy parsing
- Timeout parsing (supports integer seconds → Duration conversion)

**Configuration Structure** (`.stasis-ci.yaml`):
```yaml
image: gcc:13-bookworm
on:
  push:
    - main
    - develop
steps:
  setup:
    type: pre
    scripts:
      - "echo 'Setting up...'"
  build:
    type: exec
    scripts:
      - "g++ -o app main.cpp"
    artifacts:
      - "dist/**/*"
      - "build-info.txt"
  cleanup:
    type: post
    scripts:
      - "rm -rf tmp/"
    when: always
timeout: 1800  # 30 minutes in seconds
```

### 5. Log Streamer (`services/log_streamer.rs`)

**Responsibility**: Streams job logs to external systems.

**Key Features**:
- Real-time log streaming
- Buffering and batching
- Rate limiting
- Retry logic
- HTTP streaming to Git Server

**Key Types**:
- `LogStreamer`: Main streamer struct
- `LogEntry`: Log entry with metadata
- `LogLevel`: Log level enum

### 6. Artifact Management

**Components**:
- `artifact_collector.rs`: Discovers artifacts using glob patterns
- `artifact_compressor.rs`: Compresses artifacts (tar.gz, zip)
- `artifacts.rs`: Local filesystem storage
- `s3_artifacts.rs`: S3-compatible storage

**Artifact Flow**:
1. Post-steps specify artifact patterns in `artifacts` field
2. After job completion, artifacts are collected using glob patterns
3. Artifacts are compressed into tar.gz and zip formats
4. Compressed artifacts are uploaded to storage backend
5. Artifact metadata (name, size, checksum, URL) is stored in job state

### 7. Job Store (`stores/memory.rs`, `stores/redis.rs`)

**Responsibility**: Persists job state and history.

**Features**:
- In-memory store (default, non-persistent)
- Redis store (persistent, distributed)
- Job state tracking (pending, running, completed, failed)
- Artifact metadata storage
- Job history management

**Key Types**:
- `JobStore`: Trait for job storage
- `JobState`: Complete job state
- `ArtifactInfo`: Artifact metadata

### 8. HTTP API (`routes/api.rs`)

**Endpoints**:

**Job Management**:
- `POST /api/v1/jobs` - Submit a new job
- `GET /api/v1/jobs/{job_id}` - Get job status
- `GET /api/v1/jobs/{job_id}/logs` - Get job logs
- `GET /api/v1/jobs/{job_id}/stream` - Stream job updates (SSE)
- `POST /api/v1/jobs/{job_id}/replay` - Replay a job
- `POST /api/v1/jobs/{job_id}/retry` - Retry a failed job
- `DELETE /api/v1/jobs/{job_id}` - Cancel a running job

**Artifacts**:
- `POST /api/v1/jobs/{job_id}/artifacts` - Upload artifact
- `GET /api/v1/jobs/{job_id}/artifacts/{artifact_name}` - Download artifact
- `GET /api/v1/jobs/{job_id}/artifacts` - List artifacts

**System**:
- `GET /health` - Health check
- `GET /metrics` - Prometheus metrics
- `GET /api-docs/openapi.json` - OpenAPI specification
- `GET /api-docs` - Scalar UI documentation

### 9. Authentication (`middleware/auth.rs`)

**Features**:
- API key authentication (optional)
- Rate limiting per API key
- API key validation middleware

**Configuration**:
- `auth.enabled`: Enable/disable authentication
- `auth.api_keys`: List of valid API keys
- `auth.api_key_file`: Path to file containing API keys

### 10. Step Evaluator (`utils/step_evaluator.rs`)

**Responsibility**: Evaluates step conditions and retry policies.

**Features**:
- `if` condition evaluation (simple boolean expressions)
- `when` condition evaluation (always, on_success, on_failure)
- Retry policy with exponential backoff
- Step execution context (job state, previous step results)

**Condition Types**:
- `if`: Boolean expression (e.g., `"CI_BRANCH == 'main'"`)
- `when`: Enum (always, on_success, on_failure)
- `retry`: Retry policy with max attempts and delay

## Data Models

### Core Types (`models/types.rs`)

**JobContext**: Complete job execution context
```rust
pub struct JobContext {
    pub job_id: Uuid,
    pub run_id: Uuid,
    pub workspace_path: PathBuf,
    pub config: RunnerConfig,
    pub repository: RepositoryInfo,
    pub trigger: TriggerInfo,
}
```

**RunnerConfig**: Parsed `.stasis-ci.yaml` configuration
```rust
pub struct RunnerConfig {
    pub image: DockerImage,
    pub on: TriggerConditions,
    pub steps: HashMap<String, Step>,
    pub global_env: HashMap<String, String>,
    pub timeout: Option<Duration>,
}
```

**Step**: Individual step configuration
```rust
pub struct Step {
    pub step_type: StepType,  // Pre, Exec, Post
    pub scripts: Vec<String>,
    pub envs: HashMap<String, String>,
    pub working_directory: Option<PathBuf>,
    pub timeout: Option<Duration>,
    pub continue_on_error: bool,
    pub shell: Shell,
    pub if_condition: Option<String>,
    pub when: Option<WhenCondition>,
    pub retry: Option<RetryPolicy>,
    pub artifacts: Vec<String>,  // Glob patterns for artifact collection
}
```

**JobState**: Current job state
```rust
pub struct JobState {
    pub job_id: Uuid,
    pub run_id: Uuid,
    pub status: JobStatus,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub result: Option<JobResult>,
    pub artifacts: Vec<ArtifactInfo>,
}
```

**ArtifactInfo**: Artifact metadata
```rust
pub struct ArtifactInfo {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub checksum: String,
    pub url: Option<String>,  // S3 URL if using S3 storage
}
```

## Configuration

### Configuration File (`config.yaml`)

**Server Configuration**:
- `server.host`: HTTP server bind address
- `server.port`: HTTP server port
- `server.metrics_port`: Prometheus metrics port

**Executor Configuration**:
- `executor.max_concurrent_jobs`: Maximum concurrent job executions
- `executor.workspace_root`: Workspace root directory
- `executor.cache_root`: Cache root directory
- `executor.docker.socket`: Docker socket path
- `executor.resources`: CPU, memory, storage, PIDs limits
- `executor.timeouts`: Default, max, and git clone timeouts

**Storage Configuration**:
- `store.store_type`: "memory" or "redis"
- `store.redis_url`: Redis connection URL (if using Redis)
- `store.max_history`: Maximum job history to keep

**Artifact Storage**:
- `artifacts.storage_type`: "local" or "s3"
- `artifacts.s3.*`: S3 configuration (bucket, region, credentials, etc.)

**Authentication**:
- `auth.enabled`: Enable API key authentication
- `auth.api_keys`: List of valid API keys
- `auth.rate_limit_per_minute`: Rate limit per API key

## Docker Setup

### Development Environment (`docker-compose.yml`)

**Services**:
- `ci-runner-dev`: Main CI runner service
- `redis`: Redis service (optional, for persistent storage)

**Volumes**:
- `ci-workspaces`: Shared workspace volume (mounted at `/app/workspaces` in ci-runner-dev, `/workspace` in job containers)
- `ci-cache`: Cache volume
- `ci_runner_target`: Build cache volume

**Networks**:
- `ci-network`: Docker network for job containers

### Volume Mount Strategy

**Current Setup**:
- ci-runner-dev container: `ci-workspaces:/app/workspaces`
- Job containers: `ci-workspaces:/workspace`

**Issue**: Files cloned to `/app/workspaces/{job_id}` in ci-runner-dev should be visible at `/workspace/{job_id}` in job containers, but currently directories appear empty. This is under investigation.

**Workaround**: Currently creating directories in job containers if they don't exist, but files aren't being found.

## Current State & Known Issues

### Working Features

✅ HTTP API for job submission  
✅ Docker container execution  
✅ Git repository cloning  
✅ Log streaming  
✅ Artifact collection and compression  
✅ Job replay/retry  
✅ Server-Sent Events (SSE)  
✅ Conditional steps  
✅ OpenAPI documentation  
✅ Prometheus metrics  
✅ S3 artifact storage  
✅ Redis job storage  

### Known Issues

🔴 **Volume Mount Issue**: Files cloned to `/app/workspaces/{job_id}` in ci-runner-dev aren't visible at `/workspace/{job_id}` in job containers. The directories exist but are empty.

**Symptoms**:
- Diagnostic shows workspace directory doesn't exist initially
- After creating directory, it's empty
- Steps fail with "file not found" errors

**Investigation**:
- Files are successfully cloned (logs confirm 10+ files)
- Volume is mounted correctly
- Directory structure exists in volume
- But files aren't accessible in job containers

**Potential Causes**:
- Volume synchronization timing issue
- Path mapping mismatch
- Docker volume mount behavior on macOS (Docker Desktop)

### Recent Changes

- Migrated from RabbitMQ to HTTP API
- Added artifact collection and compression
- Implemented S3 artifact storage
- Added job replay/retry endpoints
- Implemented SSE for real-time updates
- Added conditional step execution
- Removed `working_dir` from container config to prevent empty directory creation
- Added comprehensive logging for debugging

## Development Workflow

### Setup

1. **Install Dependencies**:
   ```bash
   make setup
   ```

2. **Start Development Environment**:
   ```bash
   make dev-up
   ```

3. **View Logs**:
   ```bash
   make dev-logs
   ```

### Testing

**Submit a Job**:
```bash
curl -X POST http://localhost:8080/api/v1/jobs \
  -H "Content-Type: application/json" \
  -H "X-API-Key: your-api-key" \
  -d @examples/go-binary.yaml
```

**Check Job Status**:
```bash
curl http://localhost:8080/api/v1/jobs/{job_id}
```

**Stream Job Updates**:
```bash
curl http://localhost:8080/api/v1/jobs/{job_id}/stream
```

**View Metrics**:
```bash
curl http://localhost:8080/metrics
```

**View API Documentation**:
```bash
open http://localhost:8080/api-docs
```

### Example Configurations

See `examples/` directory for example `.stasis-ci.yaml` files:
- `flutter-apk.yaml`: Building Flutter APK
- `go-binary.yaml`: Compiling Go binaries for multiple platforms
- `java-docker.yaml`: Building and pushing Docker images
- `npm-publish.yaml`: Publishing NPM packages

## Dependencies

### Key Dependencies

- **tokio**: Async runtime
- **actix-web**: HTTP server framework
- **bollard**: Docker API client
- **serde/serde_yaml**: Serialization
- **uuid**: Unique identifiers
- **chrono**: Date/time handling
- **tracing**: Structured logging
- **prometheus**: Metrics
- **reqwest**: HTTP client
- **redis**: Redis client
- **aws-sdk-s3**: S3 storage
- **utoipa**: OpenAPI documentation
- **glob/regex**: Pattern matching for artifacts

## Future Improvements

- [ ] Fix volume mount issue for workspace files
- [ ] Add job tags/labels support
- [ ] Implement job export/import functionality
- [ ] Add job comments/annotations
- [ ] Improve error messages and diagnostics
- [ ] Add more comprehensive tests
- [ ] Implement job dependencies
- [ ] Add webhook support for job events
- [ ] Implement job templates
- [ ] Add support for matrix builds

## Troubleshooting

### Common Issues

**Issue**: Job fails with "file not found"  
**Solution**: Check volume mount configuration and ensure files are cloned before container starts

**Issue**: Artifacts not collected  
**Solution**: Verify artifact patterns in `.stasis-ci.yaml` match actual file paths

**Issue**: Container creation fails  
**Solution**: Check Docker socket permissions and network configuration

**Issue**: Redis connection fails  
**Solution**: Verify Redis is running and connection URL is correct

**Issue**: S3 upload fails  
**Solution**: Check S3 credentials and bucket permissions

## Contact & Support

For issues, questions, or contributions, please refer to the project repository.

