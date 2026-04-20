#!/bin/bash

# be strict
set -euo pipefail

# Name for this node, with suffix taken from the first argument
# or `bridge-0` if not provided
NODE_NAME="bridge-${1:-0}"
# a private local network
P2P_NETWORK="private"
# a bridge node configuration directory
CONFIG_DIR="$CELESTIA_HOME"
# directory and the files shared with the validator node
CREDENTIALS_DIR="/credentials"
# node credentials
NODE_KEY_FILE="$CREDENTIALS_DIR/$NODE_NAME.key"
NODE_JWT_FILE="$CREDENTIALS_DIR/$NODE_NAME.jwt"
# directory where validator will write the genesis hash
GENESIS_DIR="/genesis"
GENESIS_HASH_FILE="$GENESIS_DIR/genesis_hash"

# Wait for the validator to set up and provision us via shared dirs
wait_for_provision() {
  echo "Waiting for the validator node to start..."
  while [[ ! ( -e "$GENESIS_HASH_FILE" && -e "$NODE_KEY_FILE" ) ]]; do
    sleep 0.5
  done

  echo "Waiting for gRPC port to start"
  until grpcurl -plaintext validator:9090 list; do
    sleep 0.5
  done

  sleep 5 # let the validator finish setup
  echo "Validator is ready"
}

# Import the test account key shared by the validator
import_shared_key() {
  echo "Importing key: name=$NODE_NAME file=$NODE_KEY_FILE"
  echo "password" | cel-key import "$NODE_NAME" "$NODE_KEY_FILE" \
    --keyring-backend="test" \
    --p2p.network "$P2P_NETWORK" \
    --node.type bridge
}

add_trusted_genesis_and_set_network() {
  local genesis_hash

  genesis_hash="$(cat "$GENESIS_HASH_FILE")"
  # and make it trusted in the node's config
  echo "Trusting a genesis: $genesis_hash"
  sed -i "s/TrustedHash = .*/TrustedHash = \"$genesis_hash\"/" "$CONFIG_DIR/config.toml"
  sed -i "s/Address = \"localhost\"/Address = \"0.0.0.0\"/" "$CONFIG_DIR/config.toml"
  sed -i "s/SkipAuth = false/SkipAuth = true/" "$CONFIG_DIR/config.toml"
  # celestia-node requires setting custom network params through env
  export CELESTIA_CUSTOM="$P2P_NETWORK:$genesis_hash"
}

write_jwt_token() {
  echo "Saving JWT token to $NODE_JWT_FILE"
  celestia bridge auth admin --p2p.network "$P2P_NETWORK" > "$NODE_JWT_FILE"
  echo "JWT token has been saved"
}

main() {
  echo "Initializing a bridge node..."
  # Wait for a validator
  wait_for_provision
  # Import the key with the coins
  import_shared_key
  # Initialize the bridge node
  celestia bridge init --p2p.network "$P2P_NETWORK"
  # Trust the private blockchain
  add_trusted_genesis_and_set_network
  # Update the JWT token
  write_jwt_token
  # Start the bridge node
  ping -c 3 validator
  echo "Configuration finished. Running a bridge node..."
  celestia bridge start \
    --core.ip validator \
    --keyring.keyname "$NODE_NAME" \
    --p2p.network "$P2P_NETWORK" \
    --log.level.module share/discovery:error \
    --log.level.module module/p2p:fatal \
    --rpc.addr "0.0.0.0"
}

main
