#!/usr/bin/env bash
set -euo pipefail

GENESIS_JSON="${HIVE_GENESIS_PATH:-/genesis.json}"
GENESIS_TEMPLATE_DIR="${SOV_HIVE_GENESIS_TEMPLATE_DIR:-/opt/sov/hive/genesis-template}"
GENESIS_OUTPUT_DIR="${SOV_HIVE_GENESIS_OUTPUT_DIR:-/tmp/sov-hive-genesis}"
ROLLUP_CONFIG_PATH="${SOV_HIVE_ROLLUP_CONFIG_PATH:-/opt/sov/hive/mock_nomt_rollup_config.toml}"
ROLLUP_BIN="${SOV_HIVE_ROLLUP_BIN:-/opt/sov/bin/sov-demo-rollup}"
ENGINE_STUB_BIN="${SOV_HIVE_ENGINE_STUB_BIN:-/opt/sov/hive/engine_stub.py}"

if [[ ! -f "${GENESIS_JSON}" ]]; then
  echo "Expected geth-style genesis at ${GENESIS_JSON}" >&2
  exit 1
fi

# These inputs are part of Hive's generic eth1 lifecycle. For rpc-compat first pass,
# we intentionally do not import block data from them yet.
if [[ -f /chain.rlp ]]; then
  echo "Ignoring /chain.rlp in first-pass rpc-compat mode" >&2
fi
if [[ -d /blocks ]] && compgen -G "/blocks/*.rlp" > /dev/null; then
  echo "Ignoring /blocks/*.rlp in first-pass rpc-compat mode" >&2
fi

mkdir -p /hive-data
rm -rf "${GENESIS_OUTPUT_DIR}"

python3 /opt/sov/hive/genesis_adapter.py \
  "${GENESIS_JSON}" \
  "${GENESIS_TEMPLATE_DIR}" \
  "${GENESIS_OUTPUT_DIR}"

CHAIN_ID="$(tr -d '\n' < "${GENESIS_OUTPUT_DIR}/chain_id.txt")"
if [[ -z "${CHAIN_ID}" ]]; then
  echo "Failed to determine chain id from adapted genesis" >&2
  exit 1
fi

export SOV_TEST_CONST_OVERRIDE_CHAIN_ID="${CHAIN_ID}"
export RUST_LOG="${RUST_LOG:-info}"
export NO_COLOR="${NO_COLOR:-1}"
export CLICOLOR="${CLICOLOR:-0}"
export CLICOLOR_FORCE="${CLICOLOR_FORCE:-0}"

echo "Starting engine stub on :8551" >&2
python3 "${ENGINE_STUB_BIN}" &
ENGINE_PID=$!

cleanup() {
  kill "${ENGINE_PID}" 2>/dev/null || true
  wait "${ENGINE_PID}" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

echo "Starting sov-demo-rollup (mock DA + NOMT) on :8545" >&2
exec "${ROLLUP_BIN}" \
  --da-layer mock \
  --storage nomt \
  --rollup-config-path "${ROLLUP_CONFIG_PATH}" \
  --genesis-config-dir "${GENESIS_OUTPUT_DIR}"
