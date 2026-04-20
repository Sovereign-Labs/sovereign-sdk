#!/bin/bash
set -euo pipefail

if [ $# -ne 1 ]; then
    echo "Usage: $0 <path-to-rollup-config.toml>"
    exit 1
fi

CONFIG_PATH="$1"

if [ ! -f "$CONFIG_PATH" ]; then
    echo "Config file not found: $CONFIG_PATH"
    exit 1
fi

if ! command -v yq >/dev/null 2>&1; then
    echo "yq not found, installing..."
    YQ_BIN=/usr/local/bin/yq
    sudo curl -fsSL https://github.com/mikefarah/yq/releases/latest/download/yq_linux_amd64 -o "$YQ_BIN"
    sudo chmod +x "$YQ_BIN"
fi

STORAGE_PATH=$(yq -p toml -oy '.storage.path' "$CONFIG_PATH")
DA_CONN=$(yq -p toml -oy '.da.connection_string' "$CONFIG_PATH")
SEQUENCER_ADDR=$(yq -p toml -oy '.sequencer.preferred.postgres_config.postgres_connection_string' "$CONFIG_PATH")

# Extract sqlite file path from connection string like "sqlite:///mnt/da/demo_mock_da.sqlite?mode=rwc"
DA_PATH="${DA_CONN#sqlite://}"
DA_PATH="${DA_PATH%%\?*}"

echo "Cleaning rollup dbs"
echo "  storage.path       = $STORAGE_PATH"
echo "  mock_da path       = $DA_PATH"
echo "  sequencer postgres = $SEQUENCER_ADDR"

if [ -n "$STORAGE_PATH" ] && [ "$STORAGE_PATH" != "null" ]; then
    rm -rf "${STORAGE_PATH:?}"/*
fi

if [ -n "$DA_PATH" ] && [ "$DA_PATH" != "null" ]; then
    rm -rf "$DA_PATH"*
fi

sudo rm -f /var/log/rollup.log

echo "Cleaning sequencer's postgresql"
psql -v ON_ERROR_STOP=1 "$SEQUENCER_ADDR" <<EOF
DROP SCHEMA public CASCADE;
CREATE SCHEMA public;
GRANT ALL ON SCHEMA public TO postgres;
GRANT ALL ON SCHEMA public TO public;
EOF