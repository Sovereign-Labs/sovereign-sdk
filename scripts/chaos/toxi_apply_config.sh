#!/usr/bin/env bash
# toxi_apply_config.sh — populate the sovereign-soak toxiproxy proxies.
#
# Creates (or replaces) seven proxies via Toxiproxy admin API POST /populate:
#
#   postgres_1       :5433  → ${POSTGRES_UPSTREAM}     (primary  rollup state DB)
#   postgres_2       :5434  → ${POSTGRES_UPSTREAM}     (replica  rollup state DB)
#   postgres_3       :5435  → ${POSTGRES_UPSTREAM}     (auxiliary slot)
#   celestia_rpc_1   :26678 → ${CELESTIA_RPC_UPSTREAM} (primary)
#   celestia_rpc_2   :26679 → ${CELESTIA_RPC_UPSTREAM} (replica)
#   celestia_grpc_1  :9091  → ${CELESTIA_GRPC_UPSTREAM} (primary)
#   celestia_grpc_2  :9092  → ${CELESTIA_GRPC_UPSTREAM} (replica)
#
# Defaults are baremetal (loopback). For docker, override:
#
#   LISTEN_ADDR=0.0.0.0 \
#     POSTGRES_UPSTREAM=host.docker.internal:5432 \
#     ./toxi_apply_config.sh
#
# Env vars (all optional):
#   TOXIPROXY_HOST          admin API host:port      default 127.0.0.1:8474
#   LISTEN_ADDR             listen interface         default 127.0.0.1
#   POSTGRES_UPSTREAM       host:port for postgres   default 127.0.0.1:5432
#   CELESTIA_RPC_UPSTREAM   host:port for cel RPC    default da-private.celestia-mocha.com:26658
#   CELESTIA_GRPC_UPSTREAM  host:port for cel gRPC   default rpc-private.celestia-mocha.com:9090
#   POSTGRES_{1,2,3}_PORT, CELESTIA_RPC_{1,2}_PORT, CELESTIA_GRPC_{1,2}_PORT
#                           per-proxy listen port overrides (defaults above)

set -euo pipefail

TOXIPROXY_HOST="${TOXIPROXY_HOST:-127.0.0.1:8474}"
LISTEN_ADDR="${LISTEN_ADDR:-127.0.0.1}"
POSTGRES_UPSTREAM="${POSTGRES_UPSTREAM:-127.0.0.1:5432}"
CELESTIA_RPC_UPSTREAM="${CELESTIA_RPC_UPSTREAM:-da-private.celestia-mocha.com:26658}"
CELESTIA_GRPC_UPSTREAM="${CELESTIA_GRPC_UPSTREAM:-rpc-private.celestia-mocha.com:9090}"

POSTGRES_1_PORT="${POSTGRES_1_PORT:-5433}"
POSTGRES_2_PORT="${POSTGRES_2_PORT:-5434}"
POSTGRES_3_PORT="${POSTGRES_3_PORT:-5435}"
CELESTIA_RPC_1_PORT="${CELESTIA_RPC_1_PORT:-26678}"
CELESTIA_RPC_2_PORT="${CELESTIA_RPC_2_PORT:-26679}"
CELESTIA_GRPC_1_PORT="${CELESTIA_GRPC_1_PORT:-9091}"
CELESTIA_GRPC_2_PORT="${CELESTIA_GRPC_2_PORT:-9092}"

die() { echo "error: $*" >&2; exit 1; }

for bin in curl jq; do
  command -v "${bin}" >/dev/null || die "${bin} not found in PATH"
done

URL="http://${TOXIPROXY_HOST}"

curl -fsS --connect-timeout 5 "${URL}/proxies" >/dev/null \
  || die "toxiproxy admin API unreachable at ${URL} (start toxiproxy-server first)"

proxy() {
  local name="$1" port="$2" upstream="$3"
  jq -n \
    --arg name "${name}" \
    --arg listen "${LISTEN_ADDR}:${port}" \
    --arg upstream "${upstream}" \
    '{name: $name, listen: $listen, upstream: $upstream, enabled: true}'
}

payload="$(jq -s '.' <(
  proxy postgres_1      "${POSTGRES_1_PORT}"     "${POSTGRES_UPSTREAM}"
  proxy postgres_2      "${POSTGRES_2_PORT}"     "${POSTGRES_UPSTREAM}"
  proxy postgres_3      "${POSTGRES_3_PORT}"     "${POSTGRES_UPSTREAM}"
  proxy celestia_rpc_1  "${CELESTIA_RPC_1_PORT}" "${CELESTIA_RPC_UPSTREAM}"
  proxy celestia_rpc_2  "${CELESTIA_RPC_2_PORT}" "${CELESTIA_RPC_UPSTREAM}"
  proxy celestia_grpc_1 "${CELESTIA_GRPC_1_PORT}" "${CELESTIA_GRPC_UPSTREAM}"
  proxy celestia_grpc_2 "${CELESTIA_GRPC_2_PORT}" "${CELESTIA_GRPC_UPSTREAM}"
))"

echo "Populating toxiproxy at ${URL} ..."
curl -fsS -H 'Content-Type: application/json' -X POST \
  -d "${payload}" "${URL}/populate" >/dev/null

echo "Done. Current proxies:"
curl -fsS "${URL}/proxies" \
  | jq -r 'to_entries | sort_by(.key)[] | "  \(.key)\t\(.value.listen)\t→ \(.value.upstream)"'
