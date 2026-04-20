#!/usr/bin/env bash

set -euo pipefail

if [ $# -lt 1 ]; then
    echo "Usage: $0 <api_url> [runtime=demo-celestia] [num_workers=20]" >&2
    exit 1
fi

API_URL="$1"
RUNTIME="${2:-demo-celestia}"
NUM_WORKERS="${3:-20}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
GENERATOR="$PROJECT_ROOT/target/release/generator"

export NO_COLOR=1

echo "[$(date -Iseconds)] Building generator (no-op if up-to-date)..."
(cd "$PROJECT_ROOT" && cargo build --release -p sov-soak-testing --bin generator)

ITER=0
while true; do
    ITER=$((ITER + 1))
    SALT=$(date +%s)
    LOG_FILE="generator-${RUNTIME}-${ITER}.log"
    echo "[$(date -Iseconds)] Starting generator (iter=$ITER, salt=$SALT) -> $LOG_FILE"
    if "$GENERATOR" \
        --runtime="$RUNTIME" \
        --api-url="$API_URL" \
        --num-workers="$NUM_WORKERS" \
        --salt="$SALT" \
        --validity-profile=clean \
        --tx-type=bank \
        --restart-after-seconds=21600 \
        >"$LOG_FILE" 2>&1; then
        echo "[$(date -Iseconds)] Generator exited cleanly, stopping loop"
        break
    fi
    echo "[$(date -Iseconds)] Generator crashed (see $LOG_FILE), restarting with new salt..."
done
