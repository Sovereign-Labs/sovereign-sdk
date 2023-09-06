.PHONY: start start-existing start-new config submit-txn build-docker-image

CONTAINER_NAME=avail-light
IMAGE_NAME=avail-light
TEST_PRIVATE_KEY_PATH=../../demo-stf/src/sov-cli/test_data/minter_private_key.json
SOV_CLI_REL_PATH=../../../target/debug/sov-cli
APP_ID=7

ifndef SERIALIZED_BLOB_PATH
CONTENT :=
else
CONTENT := $(shell cat $(SERIALIZED_BLOB_PATH))
endif

ifeq ($(shell cat -s seed-phrase.json),)
SEED_PHRASE :=
else
SEED_PHRASE := $(shell cat seed-phrase.json | grep -o '"secretPhrase": *"[^"]*"' | cut -d '"' -f 4)
endif

key-exists: 
	@test -s seed-phrase.json || echo "Error: Call make create-new-key and replace const SEQUENCER_AVAIL_DA_ADDRESS in const-rollup-config with publicKey from the created seed-phrase.json"


create-new-key: check-container-running
	@echo "Creating new key..."
	@docker run -it --pull=always docker.io/parity/subkey:latest generate --output-type json > seed-phrase.json

create-new-app-key:
	@cd ../../avail-helper/ && cargo run --bin create_app_id -- --ws_uri wss://kate.avail.tools:443/ws --seed "$(SEED_PHRASE)"
	
check-docker:
	@command -v docker > /dev/null 2>&1 || { echo "Docker is not installed"; exit 1; }

check-container-running:
	@echo "Container $(CONTAINER_NAME) running"
	@docker ps --format '{{.Names}}' | grep -w $(CONTAINER_NAME) > /dev/null 2>&1

check-container-exists:
	@echo "Container $(CONTAINER_NAME) exists"
	@docker ps -a --format '{{.Names}}' | grep -w $(CONTAINER_NAME) > /dev/null 2>&1

build-docker-image:
	@docker build --platform linux/amd64 --build-arg NODE_CLIENT_URL=$$(grep -oE 'node_client_url = "(.*)"$$' rollup_config.toml | awk -F'"' '{print $$2}') --build-arg APP_ID=$(APP_ID) -t avail-light .

check-image-exists:
	@if docker images $(IMAGE_NAME) | awk '{print $$1}' | grep -q $(IMAGE_NAME); then \
		echo "Image $(IMAGE_NAME) exists."; \
	else \
		$(MAKE) build-docker-image; \
	fi

start-existing:
	@echo "Resuming existing container: $(CONTAINER_NAME)"
	docker start $(CONTAINER_NAME)

start-new:
	@echo "Starting new container: $(CONTAINER_NAME)"
	docker run -d --platform linux/amd64 -p 8000:7000 --name $(CONTAINER_NAME) -t $(IMAGE_NAME)

start-container: check-image-exists
	@$(MAKE) check-container-running || { $(MAKE) check-container-exists && $(MAKE) start-existing || $(MAKE) start-new; }

start: check-docker start-container key-exists

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
	@cd ../../avail-helper/ && cargo run --bin submit_tx -- --ws_uri wss://kate.avail.tools:443/ws --tx_blob $(CONTENT) --app_id $(APP_ID) --seed "$(SEED_PHRASE)"
endif

build-sov-cli:
	cd ../../demo-rollup && cargo build --bin sov-cli

test-serialize-create-token: check-container-running build-sov-cli
	$(SOV_CLI_REL_PATH) serialize-call ../../demo-stf/src/sov-cli/test_data/token_deployer_private_key.json Bank ../../demo-stf/src/sov-cli/test_data/create_token.json 0

test-build-blob-from-create-token: test-serialize-create-token
	$(SOV_CLI_REL_PATH) make-blob ../../demo-stf/src/sov-cli/test_data/create_token.dat > ../../demo-stf/src/sov-cli/test_data/test_blob.dat

test-create-token: test-build-blob-from-create-token
	$(MAKE) submit-txn SERIALIZED_BLOB_PATH=../../demo-stf/src/sov-cli/test_data/test_blob.dat

clean-rollup-db:
	$(eval path := ./$(shell awk -F'=' '/^path/ {print $$2}' rollup_config.toml | tr -d '[:space:]"\n'))
	@if [ -z "${path}" ] || [ "${path}" = "./" ]; then \
                echo "Path is empty or too short, not safe to remove"; \
                exit 1; \
        fi
	@echo removing rollup database "${path}"
	rm -rf "${path}"
