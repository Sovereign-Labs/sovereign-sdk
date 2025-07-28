.PHONY: clean start stop print-jwt-token logs

PROJECT_ROOT := $(shell git rev-parse --show-toplevel)
DOCKER_COMPOSE_DIR := $(PROJECT_ROOT)/docker
CREDENTIALS_DIR := $(DOCKER_COMPOSE_DIR)/celesita/credentials
DOCKER_COMPOSE := docker compose -f $(DOCKER_COMPOSE_DIR)/docker-compose.yml --profile monitoring

TEST_PRIVATE_KEY_PATH := $(PROJECT_ROOT)/examples/test-data/keys/minter_private_key.json
SOV_CLI_REL_PATH := $(PROJECT_ROOT)/target/debug/sov-cli

P2P_NETWORK := private
KEY_NAME := bridge-0
CELESTIA_START_TIME_FILE := /tmp/.sov-celestia-docker-start-timestamp.tmp
DATE_FORMAT := "+%Y-%m-%d %H:%M:%S"

JWT_TOKEN_PATH := $(CREDENTIALS_DIR)/$(KEY_NAME).jwt
get_token := $(DOCKER_COMPOSE) exec -it sequencer-0 celestia bridge auth admin --p2p.network "private"
get_token_1 := $(DOCKER_COMPOSE) exec -it sequencer-1 celestia bridge auth admin --p2p.network "private"


clean:
	# Important cleanup steps happen first
	rm -f "$(CREDENTIALS_DIR)/$(KEY_NAME).jwt"
	rm -rf "./demo_data"
	rm -rf "mock_da.sqlite"
	rm -f demo_rollup_config.toml
	rm -rf "~/.sov-cli-wallet"
	$(eval path := ./$(shell awk -F'=' '/^path/ {print $$2}' celestia_rollup_config.toml | tr -d '[:space:]"\n'))
	@if [ -z "${path}" ] || [ "${path}" = "./" ]; then \
		echo "Path is empty or too short, not safe to remove"; \
		exit 1; \
	fi
	@echo "Removing Celestia rollup database ${path}"
	rm -rf "${path}"
	$(eval path := ./$(shell awk -F'=' '/^path/ {print $$2}' mock_rollup_config.toml | tr -d '[:space:]"\n'))
	@if [ -z "${path}" ] || [ "${path}" = "./" ]; then \
		echo "Path is empty or too short, not safe to remove"; \
		exit 1; \
	fi
	@echo "Removing mock rollup database ${path}"
	rm -rf "${path}"
	@echo "Removing unsent transactions from local storage"
	@if [ -f "$(SOV_CLI_REL_PATH)" ]; then \
		$(SOV_CLI_REL_PATH) transactions clean; \
	fi

	# Attempt to stop the local Celestia network, but do not fail if Docker is not relevant
	@if command -v docker > /dev/null 2>&1 && docker info > /dev/null 2>&1; then \
		echo "Stopping local Celestia network setup"; \
		$(DOCKER_COMPOSE) down --volumes --rmi local; \
	else \
		echo "Docker daemon not running or Docker not installed, skipping Docker cleanup"; \
	fi

clean-wallet:
	@echo "Going to remove wallet from~/.sov_cli_wallet/"
	@@printf "!!! This can potentially remove the only source of private key. Are you sure? [y/N]: " && read ans && [ $${ans:-N} = y ] && rm -rf ~/.sov_cli_wallet/

start: check-docker
start:
	@timestamp=$$(date $(DATE_FORMAT)); \
	echo "$$timestamp: Starting celestia docker compose..."
	@echo $$(date +%s) > $(CELESTIA_START_TIME_FILE)
	$(DOCKER_COMPOSE) up --build --force-recreate -d --quiet-pull
	@timestamp=$$(date $(DATE_FORMAT)); \
	echo "$$timestamp: waiting for container 'sequencer-0' to become operational..."
	@while true; do \
  		timestamp=$$(date $(DATE_FORMAT)); \
		health=$$(docker inspect --format='{{.State.Health.Status}}' $$($(DOCKER_COMPOSE) ps -q sequencer-0)); \
		if [ "$$health" = "healthy" ]; then \
			echo "$$timestamp: sequencer-0 is healthy"; \
			break; \
		else \
			echo "$$timestamp: health == '$$health': Waiting for sequencer-0 to be up and running..."; \
		fi; \
		sleep 3; \
	done
	@echo "Services are ready, preparing configuration"
	@while [ -z "$$TOKEN" ]; do \
  		timestamp=$$(date $(DATE_FORMAT)); \
		echo "$$timestamp: Waiting JWT for token..."; \
		TOKEN=$$($(get_token) 2>/dev/null); \
		sleep 0.5; \
	done; \
	timestamp=$$(date $(DATE_FORMAT)); \
	echo "$$timestamp: token received: $$TOKEN";
	@TOKEN=$$($(get_token) 2>/dev/null); \
   	cat celestia_rollup_config.toml | \
   	sed 's/^\(celestia_rpc_auth_token = \)"[^"]*"/\1"'$$TOKEN'"/' > demo_rollup_config.toml;
	@start_time=$$(cat $(CELESTIA_START_TIME_FILE)); \
    end_time=$$(date +%s); \
    duration=$$((end_time - start_time)); \
    echo "Celestia docker compose has started completed."; \
    timestamp=$$(date $(DATE_FORMAT)); \
    echo "$$timestamp: Duration: $$duration seconds."

create-second-celestia-config: check-docker
create-second-celestia-config:
	@while [ -z "$$TOKEN" ]; do \
  			timestamp=$$(date $(DATE_FORMAT)); \
    		echo "$$timestamp: Waiting JWT for token..."; \
    		TOKEN=$$($(get_token_1) 2>/dev/null); \
    		sleep 1; \
    	done; \
    	timestamp=$$(date $(DATE_FORMAT)); \
    	echo "$$timestamp: token received: $$TOKEN";
	@TOKEN=$$($(get_token_1) 2>/dev/null); \
	ADDRESS=$$(cat ../../docker/credentials/bridge-1.addr); \
   	cat celestia_rollup_config.toml | \
   	sed 's/^\(celestia_rpc_auth_token = \)"[^"]*"/\1"'$$TOKEN'"/' | \
   	sed 's/^\(own_celestia_address = \)"[^"]*"/\1"'$$ADDRESS'"/' | \
   	sed 's/^\(path = \)"[^"]*"/\1"demo_data_1"/' | \
   	sed 's/^\(celestia_rpc_address = \)"[^"]*"/\1"http:\/\/127.0.0.1:36658"/' | \
   	sed 's/^bind_port = 12345/bind_port = 12355/' |  \
   	sed 's/^bind_port = 12346/bind_port = 12356/'  \
   	 > demo_rollup_config_1.toml;

clean-second-node:
	$(eval path := ./$(shell awk -F'=' '/^path/ {print $$2}' demo_rollup_config_1.toml | tr -d '[:space:]"\n'))
	@if [ -z "${path}" ] || [ "${path}" = "./" ]; then \
		echo "Path is empty or too short, not safe to remove"; \
		exit 1; \
	fi
	@echo removing celestia rollup database "${path}"
	rm -rf "${path}"


stop: check-docker
stop:
	$(DOCKER_COMPOSE) down

logs:
	$(DOCKER_COMPOSE) logs -f

print-jwt-token:
	@TOKEN=$$($(get_token) 2>/dev/null); \
	echo "Token 0: $$TOKEN"

print-jwt-token-1:
	@TOKEN=$$($(get_token_1) 2>/dev/null); \
	echo "Token 1: $$TOKEN"


check-docker:
	@command -v docker > /dev/null 2>&1 || { echo "Docker is not installed"; exit 1; }

check-sov-cli:
	@command -v $(SOV_CLI_REL_PATH) > /dev/null 2>&1 || { echo "sov-cli has not been build, run `make build`"; exit 1; }

celestia-logs: check-docker
celestia-logs:
	@echo "Showing celestia logs"
	$(DOCKER_COMPOSE) logs -f

build:
	cargo build --bins

test-create-token: check-sov-cli
test-create-token:
	$(SOV_CLI_REL_PATH) node set-url http://127.0.0.1:12346
	$(SOV_CLI_REL_PATH) keys import --skip-if-present --nickname DANGER__DO_NOT_USE_WITH_REAL_MONEY --path ../test-data/keys/token_deployer_private_key.json
	$(SOV_CLI_REL_PATH) transactions import from-file bank --chain-id 4321 --max-fee 100000000 --path ../test-data/requests/create_token.json
	@echo "Listing transactions"
	$(SOV_CLI_REL_PATH) transactions list
	@echo "Submitting a batch"
	$(SOV_CLI_REL_PATH) node submit-batch --wait-for-processing by-nickname DANGER__DO_NOT_USE_WITH_REAL_MONEY

test-create-token-second-seq: check-sov-cli
test-create-token-second-seq:
	$(SOV_CLI_REL_PATH) node set-url http://127.0.0.1:12356
	$(SOV_CLI_REL_PATH) transactions import from-file bank --chain-id 4321 --max-fee 100000000 --path ../test-data/requests/create_token.json
	@echo "Submitting a batch"
	$(SOV_CLI_REL_PATH) node submit-batch --wait-for-processing by-nickname DANGER__DO_NOT_USE_WITH_REAL_MONEY

register-second-sequencer: check-sov-cli
register-second-sequencer:
	$(SOV_CLI_REL_PATH) node set-url http://127.0.0.1:12356
	$(SOV_CLI_REL_PATH) keys import --nickname DANGER__DO_NOT_USE_WITH_REAL_MONEY --path ../test-data/keys/token_deployer_private_key.json
	$(SOV_CLI_REL_PATH) transactions import from-file sequencer-registry --chain-id 4321 --path ../test-data/requests/register_sequencer.json
	@echo "Submitting a batch"
	$(SOV_CLI_REL_PATH) node submit-batch by-nickname DANGER__DO_NOT_USE_WITH_REAL_MONEY


remove-insecure-keys: check-sov-cli
remove-insecure-keys:
	$(SOV_CLI_REL_PATH) keys remove by-address sov1lzkjgdaz08su3yevqu6ceywufl35se9f33kztu5cu2spja5hyyf


fix-readme-toc:
	doctoc README.md --github --notitle
	doctoc README_CELESTIA.md --github --notitle
