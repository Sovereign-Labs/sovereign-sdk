#!/bin/bash

# be strict
set -euo pipefail

PROJECT_DIR="$(git rev-parse --show-toplevel)"
DOCKER_DIR="$PROJECT_DIR/docker"
CREDENTIALS_DIR="$DOCKER_DIR/credentials"
DOCKER_COMPOSE_CFG="$DOCKER_DIR/docker-compose.yml"
CONFIG_TEMPLATE="$DOCKER_DIR/template.toml"
CELESTIA_RPC_PORT=26658

# get amount of running sequencers
sequencers_running() {
  docker compose -f "$DOCKER_COMPOSE_CFG" config --services | grep -c sequencer
}

# get the jwt for given sequencer
sequencer_jwt() {
  local sequencer_id="${1}"

  cat "$CREDENTIALS_DIR/bridge-${sequencer_id}.jwt"
}

# get the rpc port the sequencer's celestia node listens on
sequencer_rpc_port() {
  local sequencer_id="${1}"

  docker compose -f "$DOCKER_COMPOSE_CFG" port "sequencer-${sequencer_id}" "$CELESTIA_RPC_PORT"
}

# create a new rollup config with given id
create_rollup_config() {
  local id="${1}"
  local address
  local jwt

  jwt="$(sequencer_jwt "$id")"
  address="http://127.0.0.1:$(sequencer_rpc_port "$id")"
  storage_path="demo_data_$id"
  bind_port="1234${id}"
  target_file="rollup_config_${id}.toml"

  # use '|' in sed as the url has '/'
  sed \
    -e "s|<RPC_TOKEN>|$jwt|" \
    -e "s|<ADDRESS>|$address|" \
    -e "s|<STORAGE_PATH>|$storage_path|" \
    -e "s|12345|$bind_port|" \
    "$CONFIG_TEMPLATE" > "$target_file"
}

main() {
  local amount
  local last_idx

  # get amount of running sequencers
  amount="$(sequencers_running)"
  last_idx=$((amount - 1))

  # create the config for each rollup
  for id in $(seq 0 $last_idx); do
    create_rollup_config "$id"
  done
}

main
