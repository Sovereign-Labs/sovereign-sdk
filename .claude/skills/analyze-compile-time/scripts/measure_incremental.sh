#!/usr/bin/env bash
# Measures incremental rebuild time for a crate's tests.
#
# Usage:
#   ./measure_incremental.sh -p <crate> --file <path>
#   ./measure_incremental.sh -p sov-demo-rollup --file examples/demo-rollup/tests/all_tests.rs
#   ./measure_incremental.sh -p sov-demo-rollup --file examples/demo-rollup/tests/all_tests.rs --test evm_rpc_tests

set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

PACKAGE=""
FILE=""
TEST_TARGET=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        -p) PACKAGE="$2"; shift 2 ;;
        --file) FILE="$2"; shift 2 ;;
        --test) TEST_TARGET="--test $2"; shift 2 ;;
        *) echo "Unknown arg: $1"; exit 1 ;;
    esac
done

if [[ -z "$PACKAGE" || -z "$FILE" ]]; then
    echo "Usage: $0 -p <crate> --file <path-to-touch> [--test <test-target>]"
    echo ""
    echo "Example: $0 -p sov-demo-rollup --file examples/demo-rollup/tests/all_tests.rs"
    exit 1
fi

export SKIP_GUEST_BUILD=1

echo "=== Incremental rebuild measurement ==="
echo "Package:  $PACKAGE"
echo "Touching: $FILE"
echo "Target:   ${TEST_TARGET:-all tests}"
echo ""

touch "$FILE"

start_time=$(date +%s)
# shellcheck disable=SC2086
cargo test -p "$PACKAGE" $TEST_TARGET --no-run 2>&1
end_time=$(date +%s)

elapsed=$((end_time - start_time))

echo ""
echo "=== Results ==="
echo "Elapsed: ${elapsed}s"
echo ""
echo "Binary sizes:"
for bin in target/debug/deps/*-????????????????; do
    # Skip .d, .o, .rmeta files
    [[ "$bin" == *.d || "$bin" == *.o || "$bin" == *.rmeta ]] && continue
    # Only show large binaries (>1MB)
    size=$(stat -f%z "$bin" 2>/dev/null || stat -c%s "$bin" 2>/dev/null)
    if [ "$size" -gt 1048576 ]; then
        human=$(echo "$size" | awk '{printf "%.0fMB", $1/1048576}')
        name=$(basename "$bin" | sed 's/-[a-f0-9]\{16\}$//')
        echo "  $human  $name"
    fi
done
