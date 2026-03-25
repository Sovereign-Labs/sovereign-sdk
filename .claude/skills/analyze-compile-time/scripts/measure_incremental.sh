#!/usr/bin/env bash
# Measures incremental rebuild time for sov-demo-rollup tests.
#
# Usage:
#   ./measure_incremental.sh                          # touch all_tests.rs, build all tests
#   ./measure_incremental.sh --test evm_rpc_tests     # build only evm_rpc_tests target
#   ./measure_incremental.sh --file src/lib.rs        # touch a specific file
#   ./measure_incremental.sh --file src/lib.rs --test evm_rpc_tests

set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

FILE="examples/demo-rollup/tests/all_tests.rs"
TEST_TARGET=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        --file) FILE="$2"; shift 2 ;;
        --test) TEST_TARGET="--test $2"; shift 2 ;;
        *) echo "Unknown arg: $1"; exit 1 ;;
    esac
done

export SKIP_GUEST_BUILD=1

echo "=== Incremental rebuild measurement ==="
echo "Touching: $FILE"
echo "Target:   ${TEST_TARGET:-all tests}"
echo ""

touch "$FILE"

start_time=$(date +%s)
# shellcheck disable=SC2086
cargo test -p sov-demo-rollup $TEST_TARGET --no-run 2>&1
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
