.PHONY: start start-existing start-new config

CONTAINER_NAME=sov-celestia-local
VALIDATOR_ADDRESS=celestia1w7wcupk5gswj25c0khnkey5fwmlndx6t5aarmk
IMAGE_NAME=dubbelosix/sov-celestia-local:genesis-v0.7.1
RPC_PORT=26659
KEY_NAME=sequencer-da-address
AMOUNT=10000000utia
START_HEIGHT=1

get_address = $(shell docker exec $(CONTAINER_NAME) celestia-appd keys show $(KEY_NAME) | sed -n 's/- address: \(.*\)/\1/p')
get_auth = $(shell docker exec $(CONTAINER_NAME) /celestia bridge auth admin --node.store /bridge)

key-exists: start-container
	@docker exec $(CONTAINER_NAME) celestia-appd keys show $(KEY_NAME) || make create-new-key

create-new-key: start-container
	@echo "Creating new key..."
	@output=$$(docker exec $(CONTAINER_NAME) celestia-appd keys add $(KEY_NAME)) 
	
fund-address: start-container
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

config: start-container
ifeq ($(shell uname -s),Darwin)
	@sed -i '' 's/\(pub const SEQUENCER_DA_ADDRESS: \[u8; 47\] = \*\)b"[^"]*";/\1b"$(get_address)";/' ../const-rollup-config/src/lib.rs
	@sed -i '' 's/^\(celestia_rpc_auth_token = \)"[^"]*"/\1"$(get_auth)"/' rollup_config.toml
	@sed -i '' 's#^\(celestia_rpc_address = \)"[^"]*"#\1"http://127.0.0.1:$(RPC_PORT)"#' rollup_config.toml
	@sed -i '' 's#^\(start_height = \)[0-9]*#\1$(START_HEIGHT)#' rollup_config.toml
else
	@sed -i 's/\(pub const SEQUENCER_DA_ADDRESS: \[u8; 47\] = \*\)b"[^"]*";/\1b"$(get_address)";/' ../const-rollup-config/src/lib.rs
	@sed -i 's/^\(celestia_rpc_auth_token = \)"[^"]*"/\1"$(get_auth)"/' rollup_config.toml
	@sed -i 's#^\(celestia_rpc_address = \)"[^"]*"#\1"http://127.0.0.1:$(RPC_PORT)"#' rollup_config.toml
	@sed -i 's#^\(start_height = \)[0-9]*#\1$(START_HEIGHT)#' rollup_config.toml
endif

start: check-docker start-container validator-funded key-exists fund-address config

stop: check-docker
	docker stop $(CONTAINER_NAME)
clean: check-docker
	docker stop $(CONTAINER_NAME)
	docker rm $(CONTAINER_NAME)

auth: start
	$(get_auth)


