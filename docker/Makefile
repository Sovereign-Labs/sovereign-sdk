PROJECT_ROOT := $(shell git rev-parse --show-toplevel)
DOCKER_COMPOSE_DIR := $(PROJECT_ROOT)/docker
DOCKER_COMPOSE_CFG := $(DOCKER_COMPOSE_DIR)/docker-compose.yml
CREDENTIALS_DIR := $(PROJECT_ROOT)/examples/celestia-docker/credentials

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

restart: down up generate_configs

generate_configs: wait-compose-ready
	@$(DOCKER_COMPOSE_DIR)/generate_configs.sh

logs:
	@$(docker_compose) logs -f
