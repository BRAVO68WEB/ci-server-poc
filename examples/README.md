# CI Runner Example Configurations

This directory contains example `runner.yaml` configurations for common CI/CD use cases.

## Available Examples

### 1. Flutter APK Build (`flutter-apk.yaml`)
Builds a Flutter Android APK with optional signing.

**Features:**
- Flutter dependency installation
- Code analysis
- Unit tests
- APK building (release mode)
- Optional APK signing with keystore
- Artifact collection

**Environment Variables:**
- `KEYSTORE_FILE` - Path to keystore file (optional)
- `KEYSTORE_PASSWORD` - Keystore password (optional)
- `KEYSTORE_ALIAS` - Keystore alias (optional)
- `CI_BUILD_NUMBER` - Build number (optional)
- `CI_BUILD_NAME` - Build name/version (optional)

**Usage:**
```bash
# Copy to your Flutter project root
cp examples/flutter-apk.yaml /path/to/flutter/project/runner.yaml
```

---

### 2. Go Binary Compilation (`go-binary.yaml`)
Compiles Go binaries for multiple platforms.

**Features:**
- Go module dependency management
- Code vetting and formatting checks
- Unit tests with coverage
- Multi-platform builds (Linux AMD64/ARM64, Darwin AMD64, Windows AMD64)
- Checksum generation
- Artifact collection

**Environment Variables:**
- `GOOS` - Target operating system (default: linux)
- `GOARCH` - Target architecture (default: amd64)

**Usage:**
```bash
# Copy to your Go project root
cp examples/go-binary.yaml /path/to/go/project/runner.yaml
```

---

### 3. Java Spring Boot Docker Build (`java-docker.yaml`)
Builds and pushes Docker images for Java Spring Boot applications to Docker Hub and GHCR.

**Features:**
- Maven/Gradle build
- Unit tests
- Docker image building with multiple tags
- Push to Docker Hub
- Push to GitHub Container Registry (GHCR)
- Automatic versioning from git tags

**Prerequisites:**
- Dockerfile at project root
- Docker Hub credentials
- GitHub token with package write permissions

**Environment Variables:**
- `DOCKERHUB_USERNAME` - Docker Hub username (required)
- `DOCKERHUB_TOKEN` - Docker Hub access token (required)
- `GITHUB_TOKEN` - GitHub personal access token with `write:packages` permission (required)
- `GITHUB_REPOSITORY_OWNER` - GitHub repository owner/org (required)
- `PROJECT_NAME` - Docker image name (default: spring-boot-app)

**Usage:**
```bash
# Copy to your Spring Boot project root
cp examples/java-docker.yaml /path/to/spring-boot/project/runner.yaml
```

---

### 4. NPM Package Publishing (`npm-publish.yaml`)
Builds and publishes NPM packages to the npm registry.

**Features:**
- Dependency installation
- Linting (optional)
- Unit tests
- Build step (optional)
- Version checking (prevents duplicate publishes)
- Publishing with appropriate tags (latest/beta/next)
- Publish verification

**Prerequisites:**
- Node.js project with `package.json`
- NPM token with publish permissions

**Environment Variables:**
- `NPM_TOKEN` - NPM access token (required)
- `NPM_REGISTRY` - Custom npm registry URL (optional, defaults to registry.npmjs.org)
- `PACKAGE_NAME` - Package name override (optional, uses package.json by default)
- `CI_SKIP_LINT` - Skip linting step (default: false)
- `CI_SKIP_TESTS` - Skip tests (default: false)
- `CI_SKIP_BUILD` - Skip build step (default: false)
- `CI_SKIP_PUBLISH` - Skip publishing (default: false)
- `CI_FORCE_PUBLISH` - Force publish even if version exists (default: false)

**Usage:**
```bash
# Copy to your Node.js project root
cp examples/npm-publish.yaml /path/to/node/project/runner.yaml
```

---

## Common Configuration

All examples follow the same `runner.yaml` structure:

```yaml
image:
  name: "base-image-name"
  tag: "version"
  pull_policy: "IfNotPresent"  # or "Always" or "Never"

on:
  push:
    - "main"
    - "develop"
  tag:
    - "v*"

global_env:
  KEY: "value"

timeout: 3600  # seconds

steps:
  step_name:
    type: "pre" | "exec" | "post"
    scripts:
      - "command1"
      - "command2"
    envs:
      ENV_VAR: "value"
    continue_on_error: false
    timeout: 600
    when: "on_success" | "on_failure" | "always"
    retry:
      max_attempts: 3
      backoff_multiplier: 2.0
      initial_delay_secs: 5
```

## Step Types

- **`pre`**: Pre-execution steps (setup, dependencies, etc.)
- **`exec`**: Main execution steps (build, test, etc.)
- **`post`**: Post-execution steps (cleanup, artifact upload, etc.)

## Conditional Execution

Steps can be conditionally executed using:

- **`if_condition`**: Shell expression that must evaluate to true
- **`when`**: Run on success, failure, or always
- **`continue_on_error`**: Continue pipeline even if step fails

## Retry Policies

Steps can be retried on failure:

```yaml
retry:
  max_attempts: 3
  backoff_multiplier: 2.0  # Exponential backoff multiplier
  initial_delay_secs: 5     # Initial delay before retry
```

## Artifacts

Artifacts are automatically collected from the workspace. Configure artifact patterns in your CI server configuration.

## Customization

Feel free to customize these examples for your specific needs:

1. Adjust Docker images to match your requirements
2. Modify environment variables
3. Add or remove steps
4. Configure retry policies and timeouts
5. Add conditional logic for different branches/tags

## Contributing

If you have a useful example configuration, please contribute it! Follow the same structure and include:
- Clear comments explaining the configuration
- Required environment variables
- Prerequisites
- Usage instructions

