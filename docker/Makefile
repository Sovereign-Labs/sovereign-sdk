.PHONY: up down wait-compose-ready restart logs postgres

PROJECT_ROOT := $(shell git rev-parse --show-toplevel)
DOCKER_COMPOSE_DIR := $(PROJECT_ROOT)/docker
DOCKER_COMPOSE_CFG := $(DOCKER_COMPOSE_DIR)/docker-compose.yml

# Hyperlane CLI Docker image constants
HYPERLANE_CLI_REPO := ghcr.io/sovereign-labs/hyperlane-cli
HYPERLANE_CLI_TAG := sov-integration-3

docker_compose := docker compose -f $(DOCKER_COMPOSE_CFG)

up:
	@echo "Starting services"
	@$(docker_compose) up -d --build --force-recreate
	@echo "Waiting for services to finish setup"
	@$(docker_compose) logs -f | awk '/Provisioning finished./ {print;exit}' # exit when encounter this log entry

down:
	@echo "Shutting down services"
	@$(docker_compose) down
	@echo "Removing generated configs"
	@rm rollup_config_*.toml

# wait for the celestia network to perform setup and coins transfers
wait-compose-ready:
	@echo "Waiting for services to finish setup"
	@$(compose_logs) | awk '/Provisioning finished./ {print;exit}' # exit when encounter this log entry

restart: down up

logs:
	@$(docker_compose) logs -f

postgres:
	@echo "Cleaning up PostgreSQL containers and volumes..."
	-docker rm $$(docker ps -a | grep postgres | awk '{print $$1}')
	-docker rm $$(docker ps -a | grep pgadmin | awk '{print $$1}')
	-docker volume rm docker_postgres-data
	-docker volume rm docker_pgadmin-data
	@echo "Starting fresh PostgreSQL environment..."
	docker compose -f $(DOCKER_COMPOSE_DIR)/docker-compose.postgres.yml up

enable-chaos-std:
	./toxiproxy/remove_toxics.sh
	./toxiproxy/enable_toxics.sh
	./toxiproxy/status_chaos.sh

enable-chaos-brutal:
	./toxiproxy/remove_toxics.sh
	TIMEOUT_RATIO=0.9 LATENCY_RATIO=0.1 LIMIT_DATA_RATIO=0.4 ./toxiproxy/enable_toxics.sh
	./toxiproxy/status_chaos.sh

restart-toxiproxy:
	docker compose restart toxiproxy
	sleep 5
	TOXIPROXY_HOST="localhost" ./toxiproxy/configure.sh

# Needs docker logged in
# echo $GITHUB_TOKEN | docker login ghcr.io -u USERNAME --password-stdin
# make sure that GITHUB_TOKEN has write:packages permission for the sovereign-labs organization.
build-hyperlane-cli:
	docker build -f hyperlane/hyperlane-cli.Dockerfile --build-arg CLI_VERSION=$(HYPERLANE_CLI_TAG) -t $(HYPERLANE_CLI_REPO):$(HYPERLANE_CLI_TAG) .
	docker push $(HYPERLANE_CLI_REPO):$(HYPERLANE_CLI_TAG)
	docker run $(HYPERLANE_CLI_REPO):$(HYPERLANE_CLI_TAG) --version