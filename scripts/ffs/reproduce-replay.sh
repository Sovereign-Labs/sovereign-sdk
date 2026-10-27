#!/usr/bin/env bash
# FFS revision: 986acb0de2589430b49621c4d4fb6bc94bfa869a. Requires Linux, cc, and ptrace/io_uring access.
# FFS_BIN=/absolute/path/to/ffs bash /absolute/path/to/reproduce-replay.sh
# Optionally set FFS_PATCHED_BIN to validate replay-blocked.patch against the same failing futex trace.
set -euo pipefail
: "${FFS_BIN:?Set FFS_BIN to the absolute path of the unpatched FFS binary}"
source_dir="$(dirname "$(realpath "${BASH_SOURCE[0]}")")"
output="${FFS_OUTPUT_DIR:-$(mktemp -d /tmp/ffs-replay-repro.XXXXXX)}"
mkdir -p "$output"
output="$(realpath "$output")"
echo "Artifacts: $output"

for kind in futex uring; do
    flags=()
    if [[ "$kind" == futex ]]; then flags+=(-DREPRO_FUTEX); fi
    cc -std=c11 -O1 -g -Wall -Wextra -Werror -pthread "${flags[@]}" "$source_dir/replay-repro.c" -o "$output/$kind"
    timeout 30 "$output/$kind"
    reproduced=0
    for ((seed=0; seed<${FFS_SEEDS:-20}; seed++)); do
        prefix="$output/$kind-$seed"
        timeout 60 "$FFS_BIN" record --ticks none --seed "$seed" -o "$prefix.json" -- "$output/$kind" > "$prefix-record.log" 2>&1
        status=0
        timeout 60 "$FFS_BIN" replay "$prefix.json" > "$prefix-replay.log" 2>&1 || status=$?
        echo "$kind seed=$seed record=0 replay=$status"
        log="$(<"$prefix-replay.log")"
        if [[ "$kind" == futex && "$status" == 70 && "$log" == *'not at a syscall entry'* ]] ||
           [[ "$kind" == uring && "$status" == 5 && "$log" == *'io_uring_enter'* && "$log" == *'sched_yield'* ]]; then
            reproduced=1
            cat "$prefix-replay.log"
            if [[ "$kind" == futex && -n "${FFS_PATCHED_BIN:-}" ]]; then
                timeout 90 "$FFS_PATCHED_BIN" replay "$prefix.json" > "$prefix-fixed.log" 2>&1
                cat "$prefix-fixed.log"
            fi
            break
        fi
    done
    if [[ "$reproduced" == 0 ]]; then
        echo "Did not reproduce the expected $kind failure" >&2
        exit 1
    fi
done
