PROJECT_ROOT := $(shell git rev-parse --show-toplevel)
DOCKER_COMPOSE_DIR := $(PROJECT_ROOT)/docker
DOCKER_COMPOSE_CFG := $(DOCKER_COMPOSE_DIR)/docker-compose.yml
CREDENTIALS_DIR := $(DOCKER_COMPOSE_DIR)/credentials
TEST_PRIVATE_KEY_PATH := $(PROJECT_ROOT)/examples/test-data/keys/minter_private_key.json
SOV_CLI_REL_PATH := $(PROJECT_ROOT)/target/debug/sov-cli

# at height 3 the credits will already belong to the keys
START_HEIGHT := 3
KEY_NAME := bridge-0
RPC_PORT := 26658

# get the jwt token
get_auth = $(shell cat $(CREDENTIALS_DIR)/$(KEY_NAME).jwt)

# Docker compose commands
docker_compose := docker compose -f $(DOCKER_COMPOSE_CFG)
# start the service
compose_up := $(docker_compose) up --build --force-recreate -d
# stop the service
compose_down := $(docker_compose) down
# follow the logs
compose_logs := $(docker_compose) logs -f
# check if service is running 
# we need a bridge and validator so naive check is to have at least 2 services
compose_is_running = test 2 -le "$(shell $(docker_compose) ps --filter status=running --format json | jq 'length')"

# start the celestia network and generate a new config
start: check-docker start-network update-config
	@$(compose_logs)

# start the celestia network or attach to existing
start-network: check-docker
	@echo "Starting local celestia network setup"
	@if $(compose_is_running); then $(compose_logs); else $(compose_up); fi

# stop the celestia network
stop: check-docker
	@echo "Stopping local celestia network setup"
	@$(compose_down)

# clean up rollup data and stop the celestia network
clean: stop clean-celestia-rollup-db

# listen to celestia network logs
compose-logs: check-docker
	@echo "Showing celestia logs"
	@$(compose_logs)

# check if docker is installed
check-docker:
	@command -v docker > /dev/null 2>&1 || { echo "Docker is not installed"; exit 1; }

# check if celestia network is running
check-compose-running: check-docker
	@$(compose_is_running)

# wait for the celestia network to perform setup and coins transfers
wait-compose-ready:
	@echo "Waiting for services to finish setup"
	@$(compose_logs) | awk '/Provisioning finished./ {print;exit}' # exit when encounter this log entry

# update the rollup configuration with local network params
update-config: wait-compose-ready
ifeq ($(shell uname -s),Darwin)
	@sed -i '' 's/^\(celestia_rpc_auth_token = \)"[^"]*"/\1"$(get_auth)"/' celestia_rollup_config.toml
	@sed -i '' 's#^\(celestia_rpc_address = \)"[^"]*"#\1"http://127.0.0.1:$(RPC_PORT)"#' celestia_rollup_config.toml
	@sed -i '' 's#^\(start_height = \)[0-9]*#\1$(START_HEIGHT)#' celestia_rollup_config.toml
else
	@sed -i 's/^\(celestia_rpc_auth_token = \)"[^"]*"/\1"$(get_auth)"/' celestia_rollup_config.toml
	@sed -i 's#^\(celestia_rpc_address = \)"[^"]*"#\1"http://127.0.0.1:$(RPC_PORT)"#' celestia_rollup_config.toml
	@sed -i 's#^\(start_height = \)[0-9]*#\1$(START_HEIGHT)#' celestia_rollup_config.toml
endif

build-sov-cli:
	cargo build --bin sov-cli

test-generate-create-token-tx: wait-compose-ready build-sov-cli
	$(SOV_CLI_REL_PATH) transactions import from-file bank --chain-id 0 --path ../test-data/requests/create_token.json

set-rpc-url: build-sov-cli
	$(SOV_CLI_REL_PATH) rpc set-url http://127.0.0.1:12345

import-keys: build-sov-cli
	$(SOV_CLI_REL_PATH) keys import --nickname DANGER__DO_NOT_USE_WITH_REAL_MONEY --path ../test-data/keys/token_deployer_private_key.json

test-create-token: set-rpc-url test-generate-create-token-tx import-keys
	$(SOV_CLI_REL_PATH) rpc submit-batch by-nickname DANGER__DO_NOT_USE_WITH_REAL_MONEY

remove-insecure-keys: build-sov-cli
	$(SOV_CLI_REL_PATH) keys remove by-address sov1l6n2cku82yfqld30lanm2nfw43n2auc8clw7r5u5m6s7p8jrm4zqrr8r94

clean-celestia-rollup-db:
	$(eval path := ./$(shell awk -F'=' '/^path/ {print $$2}' celestia_rollup_config.toml | tr -d '[:space:]"\n'))
	@if [ -z "${path}" ] || [ "${path}" = "./" ]; then \
		echo "Path is empty or too short, not safe to remove"; \
		exit 1; \
	fi
	@echo removing rollup database "${path}"
	rm -rf "${path}"


clean-mock-rollup-db:
	$(eval path := ./$(shell awk -F'=' '/^path/ {print $$2}' mock_rollup_config.toml | tr -d '[:space:]"\n'))
	@if [ -z "${path}" ] || [ "${path}" = "./" ]; then \
		echo "Path is empty or too short, not safe to remove"; \
		exit 1; \
	fi
	@echo removing rollup database "${path}"
	rm -rf "${path}"
