.PHONY: start start-existing start-new config submit-txn

CONTAINER_NAME=sov-celestia-local
VALIDATOR_ADDRESS=celestia1w7wcupk5gswj25c0khnkey5fwmlndx6t5aarmk
IMAGE_NAME=dubbelosix/sov-celestia-local:genesis-v0.7.1
RPC_PORT=26658
KEY_NAME=validator
AMOUNT=10000000utia
START_HEIGHT=1
BLOB_TXN_FEE=300utia
TEST_PRIVATE_KEY_PATH=../test-data/keys/minter_private_key.json
SOV_CLI_REL_PATH=../../target/debug/sov-cli

ifndef SERIALIZED_BLOB_PATH
CONTENT :=
else
CONTENT := $(shell cat $(SERIALIZED_BLOB_PATH))
endif

get_address = $(shell docker exec $(CONTAINER_NAME) celestia-appd keys show $(KEY_NAME) | sed -n 's/- address: \(.*\)/\1/p')
get_auth = $(shell docker exec $(CONTAINER_NAME) /celestia bridge auth admin --node.store /bridge)
get_namespace = $(shell $(SOV_CLI_REL_PATH) util print-namespace)

key-exists: check-container-running
	@docker exec $(CONTAINER_NAME) celestia-appd keys show $(KEY_NAME) || make create-new-key

create-new-key: check-container-running
	@echo "Creating new key..."
	@output=$$(docker exec $(CONTAINER_NAME) celestia-appd keys add $(KEY_NAME)) 
	
fund-address: check-container-running
	@docker exec $(CONTAINER_NAME) celestia-appd query bank balances $(get_address) | grep amount || docker exec $(CONTAINER_NAME) celestia-appd tx bank send validator $(get_address) $(AMOUNT) --fees=300utia -y

check-docker:
	@command -v docker > /dev/null 2>&1 || { echo "Docker is not installed"; exit 1; }

check-container-running:
	@echo "Container $(CONTAINER_NAME) running"
	@docker ps --format '{{.Names}}' | grep -w $(CONTAINER_NAME) > /dev/null 2>&1

check-container-exists:
	@echo "Container $(CONTAINER_NAME) exists"
	@docker ps -a --format '{{.Names}}' | grep -w $(CONTAINER_NAME) > /dev/null 2>&1

start-existing:
	@echo "Resuming existing container: $(CONTAINER_NAME)"
	docker start $(CONTAINER_NAME)

start-new:
	@echo "Starting new container: $(CONTAINER_NAME)"
	docker run -d --name $(CONTAINER_NAME) --platform linux/amd64 -p 26657:26657 -p 26659:26659 -p $(RPC_PORT):$(RPC_PORT) $(IMAGE_NAME)

start-container:
	@$(MAKE) check-container-running || { $(MAKE) check-container-exists && $(MAKE) start-existing || $(MAKE) start-new; }

validator-funded:
	@until docker exec $(CONTAINER_NAME) celestia-appd query bank balances $(VALIDATOR_ADDRESS) | grep amount ; do \
		sleep 5; \
	done

config: check-container-running 
ifeq ($(shell uname -s),Darwin)
	@sed -i '' 's/^\(celestia_rpc_auth_token = \)"[^"]*"/\1"$(get_auth)"/' rollup_config.toml
	@sed -i '' 's#^\(celestia_rpc_address = \)"[^"]*"#\1"http://127.0.0.1:$(RPC_PORT)"#' rollup_config.toml
	@sed -i '' 's#^\(start_height = \)[0-9]*#\1$(START_HEIGHT)#' rollup_config.toml
else
	@sed -i 's/^\(celestia_rpc_auth_token = \)"[^"]*"/\1"$(get_auth)"/' rollup_config.toml
	@sed -i 's#^\(celestia_rpc_address = \)"[^"]*"#\1"http://127.0.0.1:$(RPC_PORT)"#' rollup_config.toml
	@sed -i 's#^\(start_height = \)[0-9]*#\1$(START_HEIGHT)#' rollup_config.toml
endif

start: check-docker start-container validator-funded key-exists fund-address config

stop: check-docker
	docker stop $(CONTAINER_NAME)
clean: check-docker
	-docker stop $(CONTAINER_NAME)
	echo 1
	-docker rm $(CONTAINER_NAME)
	echo 2
	$(MAKE) clean-rollup-db

submit-txn : check-container-running build-sov-cli
ifndef SERIALIZED_BLOB_PATH
	$(error SERIALIZED_BLOB_PATH is not defined)
else ifeq ($(wildcard $(SERIALIZED_BLOB_PATH)),)
	$(error The file $(SERIALIZED_BLOB_PATH) does not exist)
else
	@echo CONTENT:$(CONTENT)
	@docker exec $(CONTAINER_NAME) celestia-appd tx blob PayForBlobs $(get_namespace) $(CONTENT) --from $(KEY_NAME) --chain-id=test --fees=$(BLOB_TXN_FEE) -y
endif

build-sov-cli:
	cargo build --bin sov-cli

test-generate-create-token-tx: check-container-running build-sov-cli
	$(SOV_CLI_REL_PATH) transactions import from-file bank --path ../test-data/requests/create_token.json

set-rpc-url: build-sov-cli
	$(SOV_CLI_REL_PATH) rpc set-url http://localhost:12345

import-keys: build-sov-cli
	$(SOV_CLI_REL_PATH) keys import --nickname DANGER__DO_NOT_USE_WITH_REAL_MONEY --path ../test-data/keys/minter_private_key.json

test-create-token: set-rpc-url test-generate-create-token-tx import-keys
	$(SOV_CLI_REL_PATH) rpc submit-batch

remove-insecure-keys: build-sov-cli
	$(SOV_CLI_REL_PATH) keys remove by-address sov15vspj48hpttzyvxu8kzq5klhvaczcpyxn6z6k0hwpwtzs4a6wkvqwr57gc

clean-rollup-db:
	$(eval path := ./$(shell awk -F'=' '/^path/ {print $$2}' rollup_config.toml | tr -d '[:space:]"\n'))
	@if [ -z "${path}" ] || [ "${path}" = "./" ]; then \
                echo "Path is empty or too short, not safe to remove"; \
                exit 1; \
        fi
	@echo removing rollup database "${path}"
	rm -rf "${path}"
