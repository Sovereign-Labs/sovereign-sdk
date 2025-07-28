#!/bin/bash

# be strict
set -euxo pipefail

# Amount of bridge nodes to setup, taken from the first argument
# or 1 if not provided
BRIDGE_COUNT="${1:-1}"
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

# Saves the hash of the genesis node and the keys funded with the coins
# to the directory shared with the bridge node
provision_bridge_nodes() {
  local genesis_hash
  local last_node_idx=$((BRIDGE_COUNT - 1))

  # Save the genesis hash for the bridge
  genesis_hash=$(wait_for_block 1)
  echo "Saving a genesis hash to $GENESIS_HASH_FILE"
  echo "$genesis_hash" > "$GENESIS_HASH_FILE"

  # Get or create the keys for bridge nodes
  for node_idx in $(seq 0 "$last_node_idx"); do
    local bridge_name="bridge-$node_idx"
    local key_file="$CREDENTIALS_DIR/$bridge_name.key"
    local addr_file="$CREDENTIALS_DIR/$bridge_name.addr"

    if [ ! -e "$key_file" ]; then
      # if key don't exist yet, then create and export it
      # create a new key
      echo "Creating a new keys for the $bridge_name"
      celestia-appd keys add "$bridge_name" --keyring-backend "test"
      # export it
      echo "password" | celestia-appd keys export "$bridge_name" --keyring-backend "test" 2> "$key_file"
      # export associated address
      node_address "$bridge_name" > "$addr_file"
    else
      # otherwise, just import it
      echo "password" | celestia-appd keys import "$bridge_name" "$key_file" \
        --keyring-backend="test"
    fi
  done

  # Transfer the coins to bridge nodes addresses
  # Coins transfer need to be after validator registers EVM address, which happens in block 2.
  # see `setup_private_validator`
  local start_block=2

  for node_idx in $(seq 0 "$last_node_idx"); do
    # TODO: <https://github.com/celestiaorg/celestia-app/issues/2869>
    # we need to transfer the coins for each node in separate
    # block, or the signing of all but the first one will fail.
    # Unfortunately multi-send only works with >= 2 bridge nodes,
    # so we still rely on this hack.
    wait_for_block $((start_block + node_idx))

    local bridge_name="bridge-$node_idx"
    local bridge_address

    bridge_address=$(node_address "$bridge_name")

    echo "Transferring $BRIDGE_COINS coins to the $bridge_name"
    echo "y" | celestia-appd tx bank send \
      "$NODE_NAME" \
      "$bridge_address" \
      "$BRIDGE_COINS" \
      --fees 21000utia
  done

  # !! This is the last log entry that indicates the setup has finished for all the nodes
  echo "Provisioning finished."
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

  echo "~~~~~~~~~~~~~~~~"
  echo "APP CONFIG:"
  cat "$CONFIG_DIR/config/app.toml"


  # TODO: convert this to dasel
  # Set proper defaults and change ports
  # If you encounter: `sed: -I or -i may not be used with stdin` on MacOS you can mitigate by installing gnu-sed
  # https://gist.github.com/andre3k1/e3a1a7133fded5de5a9ee99c87c6fa0d?permalink_comment_id=3082272#gistcomment-3082272
  sed -i'.bak' 's|"tcp://127.0.0.1:26657"|"tcp://0.0.0.0:26657"|g' "$CONFIG_DIR/config/config.toml"
  sed -i'.bak' 's|"null"|"kv"|g' "$CONFIG_DIR/config/config.toml"

  # Enable prometheus
  sed -i'.bak' 's/prometheus = false/prometheus = true/g' "$CONFIG_DIR/config/config.toml"
  sed -i'.bak' 's/prometheus_listen_addr = ":26660"/prometheus_listen_addr = "0.0.0.0:26660"/g' "$CONFIG_DIR/config/config.toml"

  # Adjusting for faster block times
  # Numbers are derived by trial and error.
  # timeout_commit = "11s"
  sed -i'.bak' 's/^timeout_commit\s*=.*/timeout_commit = "500ms"/g' "$CONFIG_DIR/config/config.toml"
  # timeout_propose = "10s"
  sed -i'.bak' 's/^timeout_propose\s*=.*/timeout_propose = "50ms"/g' "$CONFIG_DIR/config/config.toml"
  # timeout_propose_delta = "500ms"
  sed -i'.bak' 's/^timeout_propose_delta\s*=.*/timeout_propose_delta = "10ms"/g' "$CONFIG_DIR/config/config.toml"
  # timeout_prevote = "1s"
  sed -i'.bak' 's/^timeout_prevote\s*=.*/timeout_prevote = "10ms"/g' "$CONFIG_DIR/config/config.toml"
  # timeout_prevote_delta = "500ms"
  sed -i'.bak' 's/^timeout_prevote_delta\s*=.*/timeout_prevote_delta = "10ms"/g' "$CONFIG_DIR/config/config.toml"
  # timeout_precommit = "1s"
  sed -i'.bak' 's/^timeout_precommit\s*=.*/timeout_precommit = "20ms"/g' "$CONFIG_DIR/config/config.toml"
  # timeout_precommit_delta = "500ms"
  sed -i'.bak' 's/^timeout_precommit_delta\s*=.*/timeout_precomm_delta = "10ms"/g' "$CONFIG_DIR/config/config.toml"
  cat "$CONFIG_DIR/config/config.toml"
}

main() {
  # Configure stuff
  setup_private_validator
  # Spawn a job to provision a bridge node later
  provision_bridge_nodes &
  # Start the celestia-app
  echo "Configuration finished. Running a validator node..."
  celestia-appd start --api.enable --grpc.enable --force-no-bbr
}

main
