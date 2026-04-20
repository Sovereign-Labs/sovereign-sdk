#!/usr/bin/env bash
# clean_rollup_data.sh — wipe rollup state directories + sequencer postgres schema.
#
# Reads paths from a rollup TOML config:
#   storage.path
#   da.connection_string                                       (sqlite://...)
#   sequencer.preferred.postgres_config.postgres_connection_string
#
# Env:
#   MODE=baremetal|docker     baremetal (default) runs psql directly;
#                             docker runs psql inside a compose service.
#   PG_DOCKER_SERVICE         compose service name when MODE=docker (default: postgres)
#   RM_LOG                    log file to remove (default: /var/log/rollup.log; skipped if missing)
#
# Requires: yq (mikefarah/yq, parses TOML), psql, plus docker (when MODE=docker).

set -euo pipefail

die() { echo "error: $*" >&2; exit 1; }

if [ $# -ne 1 ]; then
    echo "Usage: $0 <path-to-rollup-config.toml>" >&2
    exit 1
fi

CONFIG_PATH="$1"
[ -f "$CONFIG_PATH" ] || die "Config file not found: $CONFIG_PATH"

command -v yq >/dev/null  || die "yq required (https://github.com/mikefarah/yq)"
command -v psql >/dev/null || die "psql required"

MODE="${MODE:-baremetal}"
PG_DOCKER_SERVICE="${PG_DOCKER_SERVICE:-postgres}"
RM_LOG="${RM_LOG:-/var/log/rollup.log}"

case "$MODE" in
  baremetal|docker) ;;
  *) die "MODE must be baremetal or docker (got '$MODE')" ;;
esac

if [ "$MODE" = "docker" ]; then
  command -v docker >/dev/null || die "docker required for MODE=docker"
fi

STORAGE_PATH=$(yq -p toml -oy '.storage.path' "$CONFIG_PATH")
DA_CONN=$(yq -p toml -oy '.da.connection_string' "$CONFIG_PATH")
SEQUENCER_ADDR=$(yq -p toml -oy '.sequencer.preferred.postgres_config.postgres_connection_string' "$CONFIG_PATH")

# Extract sqlite file path from connection string like "sqlite:///mnt/da/demo_mock_da.sqlite?mode=rwc"
DA_PATH="${DA_CONN#sqlite://}"
DA_PATH="${DA_PATH%%\?*}"

echo "Cleaning rollup dbs (MODE=$MODE)"
echo "  storage.path       = $STORAGE_PATH"
echo "  mock_da path       = $DA_PATH"
echo "  sequencer postgres = $SEQUENCER_ADDR"

if [ -n "$STORAGE_PATH" ] && [ "$STORAGE_PATH" != "null" ]; then
    rm -rf "${STORAGE_PATH:?}"/*
fi

if [ -n "$DA_PATH" ] && [ "$DA_PATH" != "null" ]; then
    # Explicit triplet — `rm -rf "$DA_PATH"*` would also nuke same-prefix neighbours.
    rm -f "$DA_PATH" "${DA_PATH}-shm" "${DA_PATH}-wal"
fi

if [ -f "$RM_LOG" ]; then
    rm -f "$RM_LOG"
fi

echo "Cleaning sequencer's postgresql"
psql_cmd=(psql -v ON_ERROR_STOP=1 "$SEQUENCER_ADDR")
if [ "$MODE" = "docker" ]; then
    psql_cmd=(docker compose exec -T "$PG_DOCKER_SERVICE" psql -v ON_ERROR_STOP=1 "$SEQUENCER_ADDR")
fi

"${psql_cmd[@]}" <<EOF
DROP SCHEMA public CASCADE;
CREATE SCHEMA public;
GRANT ALL ON SCHEMA public TO postgres;
GRANT ALL ON SCHEMA public TO public;
EOF
