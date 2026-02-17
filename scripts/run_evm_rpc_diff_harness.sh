#!/usr/bin/env bash

set -euo pipefail

# ---------------------------------------------------------------------------
# run_evm_rpc_diff_harness.sh
#
# Spins up Anvil + demo-rollup (with 1-tx-per-block config), runs the
# EVM RPC differential harness, then tears everything down.
# ---------------------------------------------------------------------------

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

CONFIG_FILE="$PROJECT_ROOT/examples/demo-rollup/configs/mock_rollup_config.toml"
ROLLUP_DATA_DIR="$PROJECT_ROOT/examples/demo-rollup/demo_data"
SQLITE_DA_FILE="$PROJECT_ROOT/examples/demo-rollup/mock_da.sqlite"
HARNESS_DIR="$PROJECT_ROOT/typescript/apps/evm-rpc-diff-harness"

ANVIL_PORT=8545
ROLLUP_PORT=12346

ANVIL_PID=""
ROLLUP_PID=""
CONFIG_BACKED_UP=false

# ── Cleanup ────────────────────────────────────────────────────────────────
cleanup() {
    echo ""
    echo "==> Cleaning up..."

    if [[ -n "$ROLLUP_PID" ]] && kill -0 "$ROLLUP_PID" 2>/dev/null; then
        echo "    Stopping rollup (PID $ROLLUP_PID)"
        kill "$ROLLUP_PID" 2>/dev/null || true
        wait "$ROLLUP_PID" 2>/dev/null || true
    fi

    if [[ -n "$ANVIL_PID" ]] && kill -0 "$ANVIL_PID" 2>/dev/null; then
        echo "    Stopping anvil (PID $ANVIL_PID)"
        kill "$ANVIL_PID" 2>/dev/null || true
        wait "$ANVIL_PID" 2>/dev/null || true
    fi

    if [[ "$CONFIG_BACKED_UP" == true ]] && [[ -f "${CONFIG_FILE}.bak" ]]; then
        echo "    Restoring original config"
        mv "${CONFIG_FILE}.bak" "$CONFIG_FILE"
    fi

    echo "==> Done."
}
trap cleanup EXIT

# ── 1. Patch rollup config for 1-tx-per-block ─────────────────────────────
echo "==> Patching rollup config for single-tx-per-block..."
cp "$CONFIG_FILE" "${CONFIG_FILE}.bak"
CONFIG_BACKED_UP=true

sed -i 's/^max_batch_size_bytes = .*/max_batch_size_bytes = 50/' "$CONFIG_FILE"
sed -i 's/^batch_execution_time_limit_millis = .*/batch_execution_time_limit_millis = 1/' "$CONFIG_FILE"

# ── 2. Clean rollup state ─────────────────────────────────────────────────
echo "==> Cleaning rollup state..."
rm -rf "$ROLLUP_DATA_DIR"
rm -f "$SQLITE_DA_FILE"

# ── 3. Build demo-rollup ──────────────────────────────────────────────────
echo "==> Building demo-rollup (release)..."
cargo build --release -p sov-demo-rollup --manifest-path "$PROJECT_ROOT/Cargo.toml"

# ── 4. Start Anvil ────────────────────────────────────────────────────────
echo "==> Starting Anvil on port $ANVIL_PORT..."
ANVIL_LOG=$(mktemp /tmp/anvil-XXXXXX.log)
anvil --port "$ANVIL_PORT" > "$ANVIL_LOG" 2>&1 &
ANVIL_PID=$!
echo "    Anvil PID=$ANVIL_PID  log=$ANVIL_LOG"

# ── 5. Start demo-rollup ─────────────────────────────────────────────────
echo "==> Starting demo-rollup on port $ROLLUP_PORT..."
ROLLUP_LOG=$(mktemp /tmp/rollup-XXXXXX.log)
(
    cd "$PROJECT_ROOT/examples/demo-rollup"
    "$PROJECT_ROOT/target/release/sov-demo-rollup" \
        --da-layer mock \
        --rollup-config-path configs/mock_rollup_config.toml \
        --genesis-config-dir ../test-data/genesis/demo/mock \
        > "$ROLLUP_LOG" 2>&1
) &
ROLLUP_PID=$!
echo "    Rollup PID=$ROLLUP_PID  log=$ROLLUP_LOG"

# ── 6. Wait for both to be ready ─────────────────────────────────────────
wait_for_rpc() {
    local url="$1"
    local name="$2"
    local max_attempts=60
    local attempt=0

    echo "    Waiting for $name at $url..."
    while (( attempt < max_attempts )); do
        if curl -sf -X POST "$url" \
            -H "Content-Type: application/json" \
            -d '{"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":1}' \
            > /dev/null 2>&1; then
            echo "    $name is ready."
            return 0
        fi
        sleep 1
        (( attempt++ ))
    done

    echo "ERROR: $name did not become ready after ${max_attempts}s"
    echo "Last log lines:"
    tail -20 "$3" || true
    return 1
}

wait_for_rpc "http://127.0.0.1:$ANVIL_PORT" "Anvil" "$ANVIL_LOG"
wait_for_rpc "http://127.0.0.1:$ROLLUP_PORT/rpc" "Rollup" "$ROLLUP_LOG"

# ── 7. Install harness dependencies ──────────────────────────────────────
echo "==> Installing harness dependencies..."
(cd "$HARNESS_DIR" && pnpm install)

# ── 8. Run the harness ────────────────────────────────────────────────────
echo "==> Running EVM RPC diff harness..."
(
    cd "$HARNESS_DIR"
    pnpm run compare -- \
        --anvil "http://127.0.0.1:$ANVIL_PORT" \
        --rollup "http://127.0.0.1:$ROLLUP_PORT/rpc" \
        --pk 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80
)
HARNESS_EXIT=$?

echo ""
if [[ $HARNESS_EXIT -eq 0 ]]; then
    echo "==> Harness finished successfully."
else
    echo "==> Harness exited with code $HARNESS_EXIT."
fi

echo "==> Report: $HARNESS_DIR/artifacts/report.md"
exit $HARNESS_EXIT
