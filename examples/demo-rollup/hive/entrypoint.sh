#!/usr/bin/env bash
set -euo pipefail

GENESIS_JSON="${HIVE_GENESIS_PATH:-/genesis.json}"
GENESIS_TEMPLATE_DIR="${SOV_HIVE_GENESIS_TEMPLATE_DIR:-/opt/sov/hive/genesis-template}"
GENESIS_OUTPUT_DIR="${SOV_HIVE_GENESIS_OUTPUT_DIR:-/tmp/sov-hive-genesis}"
ROLLUP_CONFIG_PATH="${SOV_HIVE_ROLLUP_CONFIG_PATH:-/opt/sov/hive/mock_rollup_config.toml}"
ROLLUP_BIN="${SOV_HIVE_ROLLUP_BIN:-/opt/sov/bin/sov-demo-rollup}"
GENESIS_ADAPTER_BIN="${SOV_HIVE_GENESIS_ADAPTER_BIN:-/opt/sov/bin/sov-hive-genesis-adapter}"
SERVICES_BIN="${SOV_HIVE_SERVICES_BIN:-/opt/sov/hive/hive_services.py}"
BACKEND_RPC_PORT="${SOV_HIVE_BACKEND_RPC_PORT:-8546}"
ROLLUP_CONFIG_RUNTIME_PATH="${SOV_HIVE_RUNTIME_CONFIG_PATH:-/tmp/sov-hive-rollup-config.toml}"
WAIT_FOR_RPC_BIN="${SOV_HIVE_WAIT_FOR_RPC_BIN:-/opt/sov/hive/wait_for_rpc.py}"

if [[ -n "${SOV_HIVE_ENGINE_STUB_BIN:-}" ]] || [[ -n "${SOV_HIVE_RPC_PROXY_BIN:-}" ]]; then
  echo "SOV_HIVE_ENGINE_STUB_BIN and SOV_HIVE_RPC_PROXY_BIN are deprecated; use SOV_HIVE_SERVICES_BIN" >&2
fi

if [[ ! -f "${GENESIS_JSON}" ]]; then
  echo "Expected geth-style genesis at ${GENESIS_JSON}" >&2
  exit 1
fi

# These inputs are part of Hive's generic eth1 lifecycle.
# We still do not import historical block data.
# /chain.rlp is used for fork schedule derivation only.
if [[ -f /chain.rlp ]]; then
  echo "Using /chain.rlp for fork schedule derivation (no historical import)" >&2
fi
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

"${GENESIS_ADAPTER_BIN}" "${ADAPTER_ARGS[@]}"

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

cp "${ROLLUP_CONFIG_PATH}" "${ROLLUP_CONFIG_RUNTIME_PATH}"
sed -Ei "s/^(bind_port[[:space:]]*=[[:space:]]*).*/\\1${BACKEND_RPC_PORT}/" "${ROLLUP_CONFIG_RUNTIME_PATH}"

cleanup() {
  kill "${ROLLUP_PID:-}" 2>/dev/null || true
  wait "${ROLLUP_PID:-}" 2>/dev/null || true
  kill "${SERVICES_PID:-}" 2>/dev/null || true
  wait "${SERVICES_PID:-}" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

echo "Starting sov-demo-rollup backend (mock DA) on :${BACKEND_RPC_PORT}" >&2
"${ROLLUP_BIN}" \
  --da-layer mock \
  --rollup-config-path "${ROLLUP_CONFIG_RUNTIME_PATH}" \
  --genesis-config-dir "${GENESIS_OUTPUT_DIR}" &
ROLLUP_PID=$!

echo "Waiting for backend RPC on :${BACKEND_RPC_PORT}" >&2
READY=0
for _ in $(seq 1 300); do
  if python3 "${WAIT_FOR_RPC_BIN}" "http://127.0.0.1:${BACKEND_RPC_PORT}/rpc" 0.5; then
    READY=1
    break
  fi
  if ! kill -0 "${ROLLUP_PID}" 2>/dev/null; then
    echo "Backend process exited before becoming ready" >&2
    exit 1
  fi
  sleep 0.1
done
if [[ "${READY}" -ne 1 ]]; then
  echo "Backend RPC did not become ready on :${BACKEND_RPC_PORT}" >&2
  exit 1
fi

echo "Starting hive services (engine stub :8551 and RPC root proxy :8545)" >&2
export SOV_HIVE_RPC_BACKEND_URL="http://127.0.0.1:${BACKEND_RPC_PORT}/rpc"
python3 "${SERVICES_BIN}" &
SERVICES_PID=$!

wait -n "${ROLLUP_PID}" "${SERVICES_PID}"
