.PHONY: dev dev-up dev-down dev-logs dev-build dev-clean test

# Development with Docker Compose
dev: dev-up

dev-up:
	@echo "Starting development environment..."
	docker compose up

dev-down:
	@echo "Stopping development environment..."
	docker compose down

dev-logs:
	docker compose logs -f ci-runner-dev

dev-build:
	@echo "Building development container..."
	docker compose build ci-runner-dev

dev-clean:
	@echo "Cleaning up development environment..."
	docker compose down -v
	docker compose rm -f

dev-restart:
	@echo "Restarting development environment..."
	docker compose restart ci-runner-dev

# Local development (without Docker)
build:
	cargo build --bin ci_runner

run:
	cargo run --bin ci_runner

test:
	cargo test

check:
	cargo check

# Setup helpers
setup-dirs:
	@mkdir -p workspaces cache secrets
	@echo "Created directories: workspaces, cache, secrets"

setup-secrets:
	@echo "dummy_token" > secrets/gs_token
	@echo "ci_user" > secrets/rabbitmq_user
	@echo "ci_pass" > secrets/rabbitmq_pass
	@echo "{}" > secrets/registry-auth.json
	@echo "dummy_vault_token" > secrets/vault_token
	@echo "Created dummy secret files"

setup: setup-dirs setup-secrets
	@echo "Development environment setup complete!"

