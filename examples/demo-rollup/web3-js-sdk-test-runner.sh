#!/bin/bash

# ---------------------------------------------------------------------------
# DESCRIPTION:
# This script checks compatibility between the local demo rollup and the
# web3-js SDK by:
#   1. Building and running the demo rollup.
#   2. Waiting for a sufficiently advanced slot number.
#   3. Installing and building the web3-js SDK.
#   4. Un-commenting integration tests and running them.
#
# IF TESTS FAIL:
#   - The schema or API may have changed and the web3-js SDK needs updating.
#   - Update the [web3-js SDK repo](https://github.com/Sovereign-Labs/sovereign-sdk-web3-js) accordingly.
#   - Then, in the "check-web3-js-sdk-integration" job (under "Checkout web3-js SDK"),
#     adjust the "ref:" to point to the updated commit/branch.
# ---------------------------------------------------------------------------

# ---------------------------------------------------------------------------
# Preparing demo rollup
cargo build
# we still use `cargo run`, to not deal with `target` folder location
cargo run >demo_rollup_log.log 2>&1 &

echo "Waiting for slot number to be greater than 1..."
iterations=100
for i in $(seq 1 $iterations); do
    response=$(curl -s -S http://localhost:12346/ledger/slots/latest 2>&1)
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
cd web3-js-sdk

pnpm install
pnpm build

pnpm exec vitest --project integration --exclude apps/** --passWithNoTests=false || {
    echo "=== Demo Rollup output ==="
    cat ../demo_rollup_log.log
    exit 1
}
echo "Integration tests passed!"
