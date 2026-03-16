#!/bin/bash

# ---------------------------------------------------------------------------
# DESCRIPTION:
# This script checks compatibility between the local demo rollup and TypeScript
# clients by:
#   1. Building and running the demo rollup.
#   2. Waiting for a sufficiently advanced slot number.
#   3. Installing and building the TypeScript workspace.
#   4. Running the workspace integration tests.
#   5. Running the viem paymaster integration package against the live node.
#
# IF TESTS FAIL:
#   - The schema or API may have changed and the web3-js SDK needs updating.
#   - Update the [web3-js SDK repo](https://github.com/Sovereign-Labs/sovereign-sdk-web3-js) accordingly.
#   - Then, in the "check-web3-js-sdk-integration" job (under "Checkout web3-js SDK"),
#     adjust the "ref:" to point to the updated commit/branch.
# ---------------------------------------------------------------------------

# ---------------------------------------------------------------------------
# Building in foreground.
cargo build
# we still use `cargo run`, to not deal with `target` folder location
cargo run >demo_rollup_log.log 2>&1 &
CARGO_PID=$!

cleanup() {
    if kill -0 $CARGO_PID 2>/dev/null; then
        kill $CARGO_PID 2>/dev/null || true
        wait $CARGO_PID 2>/dev/null || true
    fi
}

trap cleanup EXIT

# Check if the process started successfully
sleep 2
if ! kill -0 $CARGO_PID 2>/dev/null; then
    echo "Error: cargo run process failed to start or exited immediately"
    echo "=== Demo Rollup output ==="
    cat demo_rollup_log.log
    exit 1
fi
echo "Cargo process started successfully (PID: $CARGO_PID)"

# Wait for HTTP server to start
echo "Waiting for HTTP server to start..."
iterations=60
for i in $(seq 1 $iterations); do
    if grep -q "Starting HTTP server" demo_rollup_log.log; then
        echo "HTTP server started!"
        break
    fi
    # Also check if process is still running
    if ! kill -0 $CARGO_PID 2>/dev/null; then
        echo "Error: cargo run process died unexpectedly"
        echo "=== Demo Rollup output ==="
        cat demo_rollup_log.log
        exit 1
    fi
    echo "Waiting for HTTP server... ($i/$iterations)"
    sleep 1
    if [ $i -eq $iterations ]; then
        echo "Timeout waiting for HTTP server to start"
        echo "=== Demo Rollup output ==="
        cat demo_rollup_log.log
        exit 1
    fi
done

echo "Waiting for slot number to be greater than 1..."
iterations=100
for i in $(seq 1 $iterations); do
    response=$(curl -s -S http://127.0.0.1:12346/ledger/slots/latest 2>&1)
    slot_number=$(echo "$response" | jq -r '.number')
    if [ ! -z "$slot_number" ] && [ "$slot_number" -gt 1 ]; then
        echo "Rollup is ready! Slot number: $slot_number"
        break
    fi
    echo "RESPONSE: '$response'"
    echo "Waiting... ($i/$iterations)"
    sleep 2
    if [ $i -eq $iterations ]; then
        echo "Timeout waiting for Rollup to become ready"
        echo "=== Demo Rollup output ==="
        cat demo_rollup_log.log
        exit 1
    fi
done

# ---------------------------------------------------------------------------
# Actual testing is going to be run here

echo "Preparing Web3 JS SDK"
cd ../../typescript

pnpm install
pnpm build

pnpm exec vitest --project integration --exclude apps/** --passWithNoTests=false || {
    echo "=== Demo Rollup output ==="
    cat ../examples/demo-rollup/demo_rollup_log.log
    exit 1
}
echo "Integration tests passed!"

echo "Running demo-rollup viem paymaster tests"
pnpm --filter @sovereign-sdk/demo-rollup-viem-tests test:demo-rollup || {
    echo "=== Demo Rollup output ==="
    cat ../examples/demo-rollup/demo_rollup_log.log
    exit 1
}
echo "demo-rollup viem paymaster tests passed!"
