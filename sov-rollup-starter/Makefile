#TEST_PRIVATE_KEY_PATH := $(PROJECT_ROOT)/examples/test-data/keys/minter_private_key.json
PROJECT_ROOT := $(shell git rev-parse --show-toplevel)
SOV_CLI_REL_PATH := $(PROJECT_ROOT)/target/debug/starter-cli-wallet

build-sov-cli:
	cargo build --bin starter-cli-wallet

set-rpc-url: build-sov-cli
	$(SOV_CLI_REL_PATH) rpc set-url http://127.0.0.1:12345

import-keys: build-sov-cli
	$(SOV_CLI_REL_PATH) keys import --nickname DANGER__DO_NOT_USE_WITH_REAL_MONEY --path test-data/keys/token_deployer_private_key.json


test-generate-create-token-tx: build-sov-cli
	$(SOV_CLI_REL_PATH) transactions import from-file bank --path test-data/requests/create_token.json


test-create-token: set-rpc-url test-generate-create-token-tx import-keys
	$(SOV_CLI_REL_PATH) rpc submit-batch by-nickname DANGER__DO_NOT_USE_WITH_REAL_MONEY


test-bank-supply-of: 
	curl -X POST -H "Content-Type: application/json" -d '{"jsonrpc":"2.0","method":"bank_supplyOf","params":["sov1zdwj8thgev2u3yyrrlekmvtsz4av4tp3m7dm5mx5peejnesga27svq9m72"],"id":1}' http://127.0.0.1:12345 