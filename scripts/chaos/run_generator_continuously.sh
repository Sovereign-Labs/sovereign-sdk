#!/usr/bin/env bash
# run_generator_continuously.sh — keep the soak-test generator alive in a loop.
#
# Builds sov-soak-testing/generator (no-op if cached), then loops:
#   spawn generator → on crash, restart with a fresh salt; on clean exit, stop.
#
# Env:
#   LOG_DIR    where to write generator-*.log    default: cwd
#   LOG_KEEP   how many log files to retain      default: 10

set -euo pipefail

if [ $# -lt 1 ]; then
    echo "Usage: $0 <api_url> [runtime=demo-celestia] [num_workers=20]" >&2
    exit 1
fi

API_URL="$1"
RUNTIME="${2:-demo-celestia}"
NUM_WORKERS="${3:-20}"
LOG_DIR="${LOG_DIR:-.}"
LOG_KEEP="${LOG_KEEP:-10}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
GENERATOR="$PROJECT_ROOT/target/release/generator"

mkdir -p "$LOG_DIR"

export NO_COLOR=1

echo "[$(date -Iseconds)] Building generator (no-op if up-to-date)..."
(cd "$PROJECT_ROOT" && cargo build --release -p sov-soak-testing --bin generator)

prune_logs() {
    # Keep the most recent $LOG_KEEP files; delete the rest.
    # `ls -t` orders newest-first; `tail -n +N` skips the first N-1.
    local extra
    # shellcheck disable=SC2012  # ls -t is the simplest way to sort by mtime here
    extra="$(ls -t "$LOG_DIR"/generator-"${RUNTIME}"-*.log 2>/dev/null | tail -n "+$((LOG_KEEP + 1))" || true)"
    [ -n "$extra" ] && echo "$extra" | xargs rm -f --
}

ITER=0
while true; do
    ITER=$((ITER + 1))
    SALT=$(date +%s)
    LOG_FILE="$LOG_DIR/generator-${RUNTIME}-${ITER}.log"
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
    prune_logs
done
