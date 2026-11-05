.PHONY: up down wait-compose-ready restart logs postgres enable-chaos-std enable-chaos-brutal restart-toxiproxy build-hyperlane-cli build-celestia-devnet-images push-celestia-devnet-images

PROJECT_ROOT := $(shell git rev-parse --show-toplevel)
DOCKER_COMPOSE_DIR := $(PROJECT_ROOT)/docker
DOCKER_COMPOSE_CFG := $(DOCKER_COMPOSE_DIR)/docker-compose.yml

# Hyperlane CLI Docker image constants
HYPERLANE_CLI_REPO := ghcr.io/sovereign-labs/hyperlane-cli
HYPERLANE_CLI_TAG := sov-integration-12

# Celestia Docker image constants
CELESTIA_DEVNET_VALIDATOR_IMAGE := ghcr.io/sovereign-labs/celestia-validator-devnet
CELESTIA_DEVNET_VALIDATOR_TAG := v8.0.1-mocha
CELESTIA_DEVNET_BRIDGE_IMAGE := ghcr.io/sovereign-labs/celestia-bridge-devnet
CELESTIA_DEVNET_BRIDGE_TAG := v0.30.1-mocha

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

CHAOS_DIR := $(PROJECT_ROOT)/scripts/chaos
DOCKER_CHAOS_ENV := LISTEN_ADDR=0.0.0.0 POSTGRES_UPSTREAM=host.docker.internal:5432

# Steady DA latency on the primary rollup (toxi_scenario P5).
enable-chaos-std:
	$(CHAOS_DIR)/toxi_scenario.sh clear primary
	$(CHAOS_DIR)/toxi_scenario.sh scenario P5 primary
	$(CHAOS_DIR)/toxi_scenario.sh list

# Compound failure: DA latency + postgres resets (toxi_scenario P6).
enable-chaos-brutal:
	$(CHAOS_DIR)/toxi_scenario.sh clear primary
	$(CHAOS_DIR)/toxi_scenario.sh scenario P6 primary
	$(CHAOS_DIR)/toxi_scenario.sh list

restart-toxiproxy:
	docker compose restart toxiproxy
	sleep 5
	$(DOCKER_CHAOS_ENV) $(CHAOS_DIR)/toxi_apply_config.sh

# Needs docker logged in
# echo $GITHUB_TOKEN | docker login ghcr.io -u USERNAME --password-stdin
# make sure that GITHUB_TOKEN has write:packages permission for the sovereign-labs organization.
build-hyperlane-cli:
	docker build -f hyperlane/hyperlane-cli.Dockerfile --build-arg CLI_VERSION=$(HYPERLANE_CLI_TAG) -t $(HYPERLANE_CLI_REPO):$(HYPERLANE_CLI_TAG) .
	docker push $(HYPERLANE_CLI_REPO):$(HYPERLANE_CLI_TAG)
	docker run $(HYPERLANE_CLI_REPO):$(HYPERLANE_CLI_TAG) --version

build-celestia-devnet-images:
	docker build \
		--file celestia/Dockerfile.validator \
		--build-arg CELESTIA_APP_TAG=$(CELESTIA_DEVNET_VALIDATOR_TAG) \
		-t $(CELESTIA_DEVNET_VALIDATOR_IMAGE):$(CELESTIA_DEVNET_VALIDATOR_TAG) \
		celestia
	docker build \
		--file celestia/Dockerfile.bridge \
		--build-arg CELESTIA_NODE_TAG=$(CELESTIA_DEVNET_BRIDGE_TAG) \
		-t $(CELESTIA_DEVNET_BRIDGE_IMAGE):$(CELESTIA_DEVNET_BRIDGE_TAG) \
		celestia

push-celestia-devnet-images:
	docker push $(CELESTIA_DEVNET_VALIDATOR_IMAGE):$(CELESTIA_DEVNET_VALIDATOR_TAG)
	docker push $(CELESTIA_DEVNET_BRIDGE_IMAGE):$(CELESTIA_DEVNET_BRIDGE_TAG)
