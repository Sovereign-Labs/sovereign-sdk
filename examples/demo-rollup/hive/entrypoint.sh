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

# These inputs are part of Hive's generic eth1 lifecycle.
# We still do not import historical block data yet, but we do use /chain.rlp to
# infer time-based fork activation blocks so transaction validation matches fixtures.
if [[ -f /chain.rlp ]]; then
  echo "Using /chain.rlp for fork schedule derivation (no historical import yet)" >&2
  export SOV_HIVE_CHAIN_RLP_PATH="/chain.rlp"
fi
export SOV_HIVE_GENESIS_JSON_PATH="${GENESIS_JSON}"
if [[ -d /blocks ]] && compgen -G "/blocks/*.rlp" > /dev/null; then
  echo "Ignoring /blocks/*.rlp in first-pass rpc-compat mode" >&2
fi

mkdir -p /hive-data
rm -rf "${GENESIS_OUTPUT_DIR}"

ADAPTER_ARGS=(
  "${GENESIS_JSON}"
  "${GENESIS_TEMPLATE_DIR}"
  "${GENESIS_OUTPUT_DIR}"
)

if [[ -f /chain.rlp ]]; then
  ADAPTER_ARGS+=("/chain.rlp")
fi

python3 /opt/sov/hive/genesis_adapter.py "${ADAPTER_ARGS[@]}"

CHAIN_ID="$(tr -d '\n' < "${GENESIS_OUTPUT_DIR}/chain_id.txt")"
if [[ -z "${CHAIN_ID}" ]]; then
  echo "Failed to determine chain id from adapted genesis" >&2
  exit 1
fi

if [[ -n "${SOV_HIVE_COMPILED_CHAIN_ID:-}" ]] && [[ "${CHAIN_ID}" != "${SOV_HIVE_COMPILED_CHAIN_ID}" ]]; then
  echo "Genesis chainId (${CHAIN_ID}) does not match compiled CHAIN_ID (${SOV_HIVE_COMPILED_CHAIN_ID}). Rebuild image with matching HIVE_CHAIN_ID." >&2
  exit 1
fi

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
