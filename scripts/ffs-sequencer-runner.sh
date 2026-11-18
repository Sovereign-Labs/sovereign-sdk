#!/usr/bin/env bash
# Experimental Cargo/nextest target runner. Build FFS separately (tested revision: 986acb0de2589430b49621c4d4fb6bc94bfa869a).
# Select one test with cargo nextest run -p sov-sequencer --test integration -E 'test(=preferred_tx_nonce_queue::test_out_of_order_nonces)' --no-capture.
# Set CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER="bash /absolute/path/to/this/script" and FFS_BIN to the FFS executable.
# Use FFS_MODE=record (default) or explore. Replay trace.json with the same stdout/stderr descriptor types used when recording.
# Live io_uring rings cannot be checkpointed; exploration starts before storage initialization.
# Full sequencer replay is not yet reliable: the pilot hit tick mismatches and a tracer error even with FFS_TICKS=none.
set -euo pipefail

for arg in "$@"; do
    if [[ "$arg" == --list ]]; then
        exec "$@"
    fi
done

: "${FFS_BIN:?Set FFS_BIN to the absolute path of the FFS executable}"
: "${FFS_OUTPUT_DIR:?Set FFS_OUTPUT_DIR to an absolute directory for traces}"
test_name="${3:?Expected nextest arguments: BINARY --exact TEST_NAME}"

test_dir="$FFS_OUTPUT_DIR/${test_name//::/_}"
mkdir -p "$test_dir"
export TOKIO_WORKER_THREADS="${TOKIO_WORKER_THREADS:-2}"

options=(--seed "${FFS_SEED:-0}" --ticks "${FFS_TICKS:-hw}" --max-budget "${FFS_MAX_BUDGET:-100000}")
if [[ "${FFS_EXACT_PREEMPT:-0}" == 1 ]]; then
    options+=(--exact-preempt --slice-cap-ms 0)
fi

case "${FFS_MODE:-record}" in
    record)
        exec "$FFS_BIN" record "${options[@]}" -o "$test_dir/trace.json" -- "$@"
        ;;
    explore)
        exec "$FFS_BIN" explore "${options[@]}" --root start \
            --iterations "${FFS_ITERATIONS:-10}" --max-depth "${FFS_MAX_DEPTH:-1000000}" \
            --watch-count 0 --output-dir "$test_dir" -- "$@"
        ;;
    *)
        echo "FFS_MODE must be record or explore" >&2
        exit 2
        ;;
esac
