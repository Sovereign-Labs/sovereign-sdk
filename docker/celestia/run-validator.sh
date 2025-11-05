#!/bin/bash

# be strict
set -euxo pipefail

# Amount of bridge nodes to setup, taken from the first argument
# or 5 if not provided
BRIDGE_COUNT="${1:-5}"
# a private local network
P2P_NETWORK="private"
# a validator node configuration directory
CONFIG_DIR="$CELESTIA_HOME/.celestia-app"
# the names of the keys
NODE_NAME=validator-0
# amounts of the coins for the keys
BRIDGE_COINS="200000000000000utia"
VALIDATOR_COINS="1000000000000000utia"
# a directory and the files shared with the bridge nodes
CREDENTIALS_DIR="/credentials"
# directory where validator will write the genesis hash
GENESIS_DIR="/genesis"
GENESIS_HASH_FILE="$GENESIS_DIR/genesis_hash"

# Get the address of the node of given name
node_address() {
  local node_name="$1"
  local node_address

  node_address=$(celestia-appd keys show "$node_name" -a --keyring-backend="test")
  echo "$node_address"
}

# Waits for the given block to be created and returns it's hash
wait_for_block() {
  local block_num="$1"
  local block_hash=""

  # Wait for the block to be created
  while [[ -z "$block_hash" ]]; do
    # `celestia-appd` skips the block_id field so we use rest
    # `|| echo` fallbacks to an empty string in case it's not ready
    block_hash="$(curl -sS "http://localhost:26657/block?height=$block_num" 2>/dev/null | jq -r '.result.block_id.hash // ""' || echo)"
    sleep 0.1
  done

  echo "$block_hash"
}

save_genesis_hash() {
    local genesis_hash
    # Save the genesis hash for the bridge
    genesis_hash=$(wait_for_block 1)
    echo "Saving a genesis hash=$genesis_hash to $GENESIS_HASH_FILE"
    # TODO: check exit code, and compare what has been written.
    echo "$genesis_hash" > "$GENESIS_HASH_FILE"
    echo "Genesis hash has been saved"
}

fund_bridge_nodes() {
  local last_node_idx=$((BRIDGE_COUNT - 1))

  # Get or create the keys for bridge nodes
  for node_idx in $(seq 0 "$last_node_idx"); do
    local bridge_name="bridge-$node_idx"
    local key_file="$CREDENTIALS_DIR/$bridge_name.key"
    local addr_file="$CREDENTIALS_DIR/$bridge_name.addr"

    # Generate key, if necessary, otherwise just add it to keystore
    if [ ! -e "$key_file" ]; then
      # if key don't exist yet, then create and export it
      # create a new key
      echo "Creating a new keys for the $bridge_name"
      celestia-appd keys add "$bridge_name" --keyring-backend "test"
      # export it
      echo "password" | celestia-appd keys export "$bridge_name" --keyring-backend "test" > "$key_file"
      if [ ! -s "$key_file" ]; then
        echo "Exported key file for $bridge_name is empty: $key_file" >&2
        exit 1
      fi
      # export associated address
      node_address "$bridge_name" > "$addr_file"
    else
      if [ ! -s "$key_file" ]; then
        echo "Existing key file for $bridge_name is empty: $key_file" >&2
        exit 1
      fi
      # otherwise, just import it
      echo "password" | celestia-appd keys import "$bridge_name" "$key_file" \
        --keyring-backend="test"
    fi

    local bridge_address
    bridge_address=$(node_address "$bridge_name")
    celestia-appd genesis add-genesis-account "$bridge_address" "$BRIDGE_COINS"
  done
  echo "Funded bridge nodes"
}

# Set up the validator for a private alone network.
# Based on
# https://github.com/celestiaorg/celestia-app/blob/main/scripts/single-node.sh
setup_private_validator() {
  local validator_addr

  # Initialize the validator
  celestia-appd init "$P2P_NETWORK" --chain-id "$P2P_NETWORK"
  # Derive a new private key for the validator
  celestia-appd keys add "$NODE_NAME" --keyring-backend="test"
  validator_addr=$(node_address "$NODE_NAME")
  # Create a validator's genesis account for the genesis.json with an initial bag of coins
  celestia-appd genesis add-genesis-account "$validator_addr" "$VALIDATOR_COINS"
  # Generate a genesis transaction that creates a validator with a self-delegation
  celestia-appd genesis gentx "$NODE_NAME" 5000000000utia \
    --keyring-backend="test" \
    --chain-id "$P2P_NETWORK" \
    --gas-prices "1utia"
  # Add bridge node keys and fund them in genesis
  fund_bridge_nodes

  # Collect the genesis transactions and form a genesis.json
  celestia-appd genesis collect-gentxs

  # Set proper defaults and change ports
  dasel put -f "$CONFIG_DIR/config/config.toml" -t string -v 'tcp://0.0.0.0:26657' rpc.laddr
  # enable transaction indexing
  dasel put -f "$CONFIG_DIR/config/config.toml" -t string -v 'kv' tx_index.indexer

  # enable REST API
  dasel put -f "$CONFIG_DIR/config/app.toml" -t bool -v true api.enable
  dasel put -f "$CONFIG_DIR/config/app.toml" -t string -v 'tcp://0.0.0.0:1317' api.address
  # enable gRPC
  dasel put -f "$CONFIG_DIR/config/app.toml" -t bool -v true grpc.enable
  dasel put -f "$CONFIG_DIR/config/app.toml" -t string -v '0.0.0.0:9090' grpc.address

  # enable grpc-web
  dasel put -f "$CONFIG_DIR/config/app.toml" -t bool -v true grpc-web.enable
  # enable unsafe CORS since we don't do security properly in CI
  dasel put -f "$CONFIG_DIR/config/app.toml" -t bool -v true grpc-web.enable-unsafe-cors

  echo "================================"
  echo "APP CONFIG: app.toml"
  cat "$CONFIG_DIR/config/app.toml"
  echo "================================"

  # Enable prometheus
  dasel put -f "$CONFIG_DIR/config/config.toml" -t bool -v true instrumentation.prometheus
  dasel put -f "$CONFIG_DIR/config/config.toml" -t string -v '0.0.0.0:26660' instrumentation.prometheus_listen_addr

  # Adjusting for faster block times. Default values are in comments above
  # Currently disabled
#  # Numbers are derived by trial and error.
#  # timeout_commit = "11s"
#  dasel put -f "$CONFIG_DIR/config/config.toml" -t string -v '2000ms' consensus.timeout_commit
#  # timeout_propose = "10s"
#  dasel put -f "$CONFIG_DIR/config/config.toml" -t string -v '4000ms' consensus.timeout_propose
#  # timeout_propose_delta = "500ms"
#  dasel put -f "$CONFIG_DIR/config/config.toml" -t string -v '200ms' consensus.timeout_propose_delta
#  # timeout_prevote = "1s"
#  dasel put -f "$CONFIG_DIR/config/config.toml" -t string -v '400ms' consensus.timeout_prevote
#  # timeout_prevote_delta = "500ms"
#  dasel put -f "$CONFIG_DIR/config/config.toml" -t string -v '400ms' consensus.timeout_prevote_delta
#  # timeout_precommit = "1s"
#  dasel put -f "$CONFIG_DIR/config/config.toml" -t string -v '400ms' consensus.timeout_precommit
#  # timeout_precommit_delta = "500ms"
#  dasel put -f "$CONFIG_DIR/config/config.toml" -t string -v '200ms' consensus.timeout_precommit_delta

  echo "Final Private Validator Config:"
  cat "$CONFIG_DIR/config/config.toml"
  echo "End of Final Private Validator Config:"
  echo "==================================="
}

main() {
  setup_private_validator

  # Spawn a job to save genesis_hash
  save_genesis_hash &
  # Start the celestia-app
  echo "Configuration finished. Running a validator node..."
  celestia-appd start --api.enable --grpc.enable --force-no-bbr
}

main
