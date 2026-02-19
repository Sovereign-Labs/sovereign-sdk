#!/usr/bin/env bash
set -euo pipefail

RPC_URL="${RPC_URL:-http://127.0.0.1:8545}"
RPC_PATH_URL="${RPC_PATH_URL:-http://127.0.0.1:8545/rpc}"
ENGINE_URL="${ENGINE_URL:-http://127.0.0.1:8551}"
SMOKE_ADDRESS="${SMOKE_ADDRESS:-${1:-0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266}}"
CHAIN_ID_EXPECTED="${CHAIN_ID_EXPECTED:-}"

rpc_call() {
  local url="$1"
  local method="$2"
  local params="$3"
  curl -sS -H 'Content-Type: application/json' \
    --data "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"${method}\",\"params\":${params}}" \
    "${url}"
}

must_result() {
  jq -er 'if .error then .error | @json | halt_error(1) else .result end'
}

echo "[smoke] web3_clientVersion"
client_version="$(rpc_call "${RPC_URL}" "web3_clientVersion" "[]" | must_result)"
echo "  -> ${client_version}"

echo "[smoke] eth_chainId via / and /rpc"
chain_id_root="$(rpc_call "${RPC_URL}" "eth_chainId" "[]" | must_result)"
chain_id_path="$(rpc_call "${RPC_PATH_URL}" "eth_chainId" "[]" | must_result)"
if [[ "${chain_id_root}" != "${chain_id_path}" ]]; then
  echo "chainId mismatch between / (${chain_id_root}) and /rpc (${chain_id_path})" >&2
  exit 1
fi
if [[ -n "${CHAIN_ID_EXPECTED}" && "${chain_id_root}" != "${CHAIN_ID_EXPECTED}" ]]; then
  echo "chainId mismatch: expected ${CHAIN_ID_EXPECTED}, got ${chain_id_root}" >&2
  exit 1
fi
echo "  -> ${chain_id_root}"

echo "[smoke] net_version"
net_version="$(rpc_call "${RPC_URL}" "net_version" "[]" | must_result)"
echo "  -> ${net_version}"

echo "[smoke] eth_blockNumber"
block_number="$(rpc_call "${RPC_URL}" "eth_blockNumber" "[]" | must_result)"
echo "  -> ${block_number}"

echo "[smoke] eth_getBlockByNumber(latest,false)"
rpc_call "${RPC_URL}" "eth_getBlockByNumber" '["latest",false]' | must_result > /dev/null
echo "  -> ok"

echo "[smoke] eth_getBalance(${SMOKE_ADDRESS}, earliest)"
balance="$(rpc_call "${RPC_URL}" "eth_getBalance" "[\"${SMOKE_ADDRESS}\",\"earliest\"]" | must_result)"
echo "  -> ${balance}"

echo "[smoke] eth_getBlockTransactionCountByNumber(latest)"
tx_count="$(rpc_call "${RPC_URL}" "eth_getBlockTransactionCountByNumber" '["latest"]' | must_result)"
echo "  -> ${tx_count}"

echo "[smoke] engine_forkchoiceUpdatedV3 stub"
engine_resp="$(curl -sS -H 'Content-Type: application/json' \
  --data '{"jsonrpc":"2.0","id":1,"method":"engine_forkchoiceUpdatedV3","params":[{"headBlockHash":"0x0000000000000000000000000000000000000000000000000000000000000000","safeBlockHash":"0x0000000000000000000000000000000000000000000000000000000000000000","finalizedBlockHash":"0x0000000000000000000000000000000000000000000000000000000000000000"},null]}' \
  "${ENGINE_URL}")"
status="$(printf '%s' "${engine_resp}" | jq -er '.result.payloadStatus.status')"
if [[ "${status}" != "VALID" ]]; then
  echo "engine stub returned unexpected status: ${status}" >&2
  exit 1
fi
echo "  -> ${status}"

echo "[smoke] all checks passed"
