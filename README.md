# Stasis CI

A distributed CI/CD runner system built with Rust, designed to execute CI jobs in isolated Docker containers.

## Features

- **Distributed Architecture**: RabbitMQ-based job queue for scalable job processing
- **Docker Isolation**: Each job runs in an isolated Docker container
- **Git Integration**: Secure repository cloning with authentication
- **Real-time Logging**: Stream logs to Git Server in real-time
- **Job Scheduling**: Concurrent job execution with configurable limits
- **HTTP API**: Health checks, metrics, and job cancellation endpoints

## Project Structure

```
.
├── crates/
│   └── ci_runner/          # Main CI runner application
├── config.yaml             # Configuration file
├── docker-compose.yml      # Development environment
├── Dockerfile.dev          # Development Dockerfile
└── Makefile               # Development commands
```

## Quick Start

### Prerequisites

- Docker and Docker Compose
- Rust toolchain (for local development)

### Development Setup

1. **Setup directories and secrets:**
   ```bash
   make setup
   ```

2. **Start development environment:**
   ```bash
   make dev-up
   ```

3. **View logs:**
   ```bash
   make dev-logs
   ```

### Configuration

Edit `config.yaml` to configure:
- RabbitMQ connection settings
- Docker executor settings
- Git Server endpoints
- Resource limits and timeouts

## Development Commands

- `make dev-up` - Start development environment
- `make dev-down` - Stop development environment
- `make dev-logs` - View CI runner logs
- `make dev-build` - Rebuild development container
- `make dev-clean` - Clean up volumes and containers
- `make build` - Build locally (without Docker)
- `make run` - Run locally
- `make test` - Run tests
- `make check` - Check code without building