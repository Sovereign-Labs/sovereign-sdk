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
ANVIL_LOG=""
ROLLUP_LOG=""
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

    [[ -n "$ANVIL_LOG" ]] && rm -f "$ANVIL_LOG"
    [[ -n "$ROLLUP_LOG" ]] && rm -f "$ROLLUP_LOG"

    echo "==> Done."
}
trap cleanup EXIT

# ── 1. Patch rollup config for 1-tx-per-block ─────────────────────────────
echo "==> Patching rollup config for single-tx-per-block..."

# If .bak exists from a previous interrupted run, restore it first so we
# start from a known-good config.
if [[ -f "${CONFIG_FILE}.bak" ]]; then
    echo "    Restoring config from previous interrupted run"
    mv "${CONFIG_FILE}.bak" "$CONFIG_FILE"
fi

cp "$CONFIG_FILE" "${CONFIG_FILE}.bak"
CONFIG_BACKED_UP=true

sed -i 's/^max_batch_size_bytes = .*/max_batch_size_bytes = 1048576/' "$CONFIG_FILE"
# We do NOT override batch_execution_time_limit_millis here. The default (2000ms)
# is sufficient because the harness sends transactions sequentially (each call
# awaits the receipt before sending the next), so 1-tx-per-batch happens
# naturally. Setting it to 1ms causes slots to advance faster than mock DA can
# finalize, triggering 503 "sequencer overloaded" errors mid-test.

# ── 2. Clean rollup state ─────────────────────────────────────────────────
echo "==> Cleaning rollup state..."
rm -rf "$ROLLUP_DATA_DIR"
rm -f "$SQLITE_DA_FILE"

# ── 3. Build demo-rollup ──────────────────────────────────────────────────
echo "==> Building demo-rollup..."
cargo build -p sov-demo-rollup --manifest-path "$PROJECT_ROOT/Cargo.toml"

# ── 4. Kill stale processes on our ports ──────────────────────────────────
echo "==> Checking for stale processes on ports..."
for port in "$ANVIL_PORT" "$ROLLUP_PORT"; do
    if lsof -ti :"$port" > /dev/null 2>&1; then
        echo "    Killing stale process(es) on port $port"
        lsof -ti :"$port" 2>/dev/null | xargs kill 2>/dev/null || true
        sleep 0.5
    fi
done

# ── 5. Start Anvil ────────────────────────────────────────────────────────
echo "==> Starting Anvil on port $ANVIL_PORT..."
ANVIL_LOG=$(mktemp /tmp/anvil-XXXXXX.log)
anvil --port "$ANVIL_PORT" > "$ANVIL_LOG" 2>&1 &
ANVIL_PID=$!
echo "    Anvil PID=$ANVIL_PID  log=$ANVIL_LOG"

# ── 6. Start demo-rollup ─────────────────────────────────────────────────
echo "==> Starting demo-rollup on port $ROLLUP_PORT..."
ROLLUP_LOG=$(mktemp /tmp/rollup-XXXXXX.log)
(
    cd "$PROJECT_ROOT/examples/demo-rollup"
    exec "$PROJECT_ROOT/target/debug/sov-demo-rollup" \
        --da-layer mock \
        --rollup-config-path configs/mock_rollup_config.toml \
        --genesis-config-dir ../test-data/genesis/demo/mock
) > "$ROLLUP_LOG" 2>&1 &
ROLLUP_PID=$!
echo "    Rollup PID=$ROLLUP_PID  log=$ROLLUP_LOG"

# ── 7. Wait for both to be ready ─────────────────────────────────────────
wait_for_endpoint() {
    local url="$1"
    local name="$2"
    local log_file="$3"
    shift 3
    local max_attempts=60
    local attempt=0

    echo "    Waiting for $name..."
    while (( attempt < max_attempts )); do
        if curl -sf "$@" "$url" > /dev/null 2>&1; then
            echo "    $name is ready."
            return 0
        fi
        sleep 1
        (( ++attempt ))
    done

    echo "ERROR: $name did not become ready after ${max_attempts}s"
    echo "Last log lines:"
    tail -20 "$log_file" || true
    return 1
}

wait_for_endpoint "http://127.0.0.1:$ANVIL_PORT" "Anvil" "$ANVIL_LOG" \
    -X POST -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":1}'

wait_for_endpoint "http://127.0.0.1:$ROLLUP_PORT/rpc" "Rollup RPC" "$ROLLUP_LOG" \
    -X POST -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":1}'

wait_for_endpoint "http://127.0.0.1:$ROLLUP_PORT/sequencer/ready" "Rollup sequencer" "$ROLLUP_LOG"

# ── 8. Install harness dependencies ──────────────────────────────────────
echo "==> Installing harness dependencies..."
(cd "$HARNESS_DIR" && pnpm install)

# ── 9. Clear stale reports so a crash doesn't leave misleading artifacts ──
rm -f "$HARNESS_DIR/artifacts/report.json" "$HARNESS_DIR/artifacts/report.md"

# ── 10. Run the harness ───────────────────────────────────────────────────
echo "==> Running EVM RPC diff harness..."
# Use env vars instead of CLI args to avoid pnpm arg-forwarding issues
# with compound scripts ("build:contracts && tsx src/run.ts").
HARNESS_EXIT=0
(
    cd "$HARNESS_DIR"
    ANVIL_RPC_URL="http://127.0.0.1:$ANVIL_PORT" \
    ROLLUP_RPC_URL="http://127.0.0.1:$ROLLUP_PORT/rpc" \
    TEST_PRIVATE_KEY="0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80" \
    pnpm run compare
) || HARNESS_EXIT=$?

echo ""
if [[ $HARNESS_EXIT -eq 0 ]]; then
    echo "==> Harness finished successfully."
else
    echo "==> Harness exited with code $HARNESS_EXIT."
fi

echo "==> Report: $HARNESS_DIR/artifacts/report.md"

if [[ -f "$HARNESS_DIR/artifacts/report.md" ]]; then
    echo ""
    cat "$HARNESS_DIR/artifacts/report.md"
fi

exit $HARNESS_EXIT
