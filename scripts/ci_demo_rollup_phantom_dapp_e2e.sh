#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEMO_DIR="$ROOT_DIR/examples/demo-rollup"
DAPP_DIR="$DEMO_DIR/dapps/phantom"
ROLLUP_BIN="$ROOT_DIR/target/debug/sov-demo-rollup"
ROLLUP_LOG="$DEMO_DIR/demo_rollup_log.log"
ROLLUP_PID=""

cleanup() {
    if [[ -n "$ROLLUP_PID" ]]; then
        kill "$ROLLUP_PID" 2>/dev/null || true
    fi
}

trap cleanup EXIT

SOV_PROVER_MODE=skip cargo build -p sov-demo-rollup --bin sov-demo-rollup

cd "$DEMO_DIR"
rm -rf demo_data mock_da.sqlite "$ROLLUP_LOG"

SOV_PROVER_MODE=skip "$ROLLUP_BIN" >"$ROLLUP_LOG" 2>&1 &
ROLLUP_PID="$!"

for i in $(seq 1 30); do
    if curl -sf http://127.0.0.1:12346/healthcheck >/dev/null; then
        break
    fi

    if ! kill -0 "$ROLLUP_PID" 2>/dev/null; then
        echo "Demo rollup exited unexpectedly"
        cat "$ROLLUP_LOG"
        exit 1
    fi

    echo "Waiting for demo rollup... ($i/30)"
    sleep 2
done

if ! curl -sf http://127.0.0.1:12346/healthcheck >/dev/null; then
    echo "Timed out waiting for demo rollup healthcheck"
    cat "$ROLLUP_LOG"
    exit 1
fi

cd "$DAPP_DIR"
pnpm install --frozen-lockfile
pnpm exec playwright install --with-deps chromium

CI=true \
VITE_ROLLUP_URL=http://127.0.0.1:12346 \
VITE_CHAIN_ID=4321 \
VITE_SOLANA_ENDPOINT=/sequencer/accept-solana-offchain-tx \
pnpm run test:e2e
