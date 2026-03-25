#!/usr/bin/env bash
# Reports test binary sizes and codegen unit breakdown.
#
# Usage:
#   ./binary_sizes.sh              # show all test binaries
#   ./binary_sizes.sh --cgu        # also show CGU breakdown for largest binary

set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

SHOW_CGU=false
[[ "${1:-}" == "--cgu" ]] && SHOW_CGU=true

echo "=== Test binary sizes ==="
echo ""

# Find test binaries (executables, not .d/.o/.rmeta files)
found=0
for bin in target/debug/deps/*-????????????????; do
    [[ "$bin" == *.d || "$bin" == *.o || "$bin" == *.rmeta ]] && continue
    [[ -x "$bin" ]] || continue

    size=$(stat -f%z "$bin" 2>/dev/null || stat -c%s "$bin" 2>/dev/null)
    human=$(echo "$size" | awk '{
        if ($1 >= 1073741824) printf "%.1fGB", $1/1073741824
        else if ($1 >= 1048576) printf "%.0fMB", $1/1048576
        else if ($1 >= 1024) printf "%.0fKB", $1/1024
        else printf "%dB", $1
    }')
    name=$(basename "$bin" | sed 's/-[a-f0-9]\{16\}$//')
    echo "  $human	$name	$(basename "$bin")"
    found=$((found + 1))
done | sort -t'	' -k1 -rh

echo ""
echo "Total binaries: $found"

if $SHOW_CGU; then
    echo ""
    echo "=== CGU breakdown (largest binary) ==="

    # Find the largest binary
    largest=""
    largest_size=0
    for bin in target/debug/deps/*-????????????????; do
        [[ "$bin" == *.d || "$bin" == *.o || "$bin" == *.rmeta ]] && continue
        [[ -x "$bin" ]] || continue
        size=$(stat -f%z "$bin" 2>/dev/null || stat -c%s "$bin" 2>/dev/null)
        if [ "$size" -gt "$largest_size" ]; then
            largest_size=$size
            largest=$bin
        fi
    done

    if [ -n "$largest" ]; then
        hash=$(basename "$largest" | grep -o '[a-f0-9]\{16\}$')
        name=$(basename "$largest" | sed 's/-[a-f0-9]\{16\}$//')
        echo "Binary: $name ($(echo "$largest_size" | awk '{printf "%.0fMB", $1/1048576}'))"
        echo ""

        total_cgu=0
        for cgu in target/debug/deps/"$(basename "$largest")".*-cgu.*.rcgu.o; do
            [ -f "$cgu" ] || continue
            size=$(stat -f%z "$cgu" 2>/dev/null || stat -c%s "$cgu" 2>/dev/null)
            human=$(echo "$size" | awk '{printf "%.0fMB", $1/1048576}')
            cgu_num=$(echo "$cgu" | grep -o 'cgu\.[0-9]*' | grep -o '[0-9]*')
            echo "  CGU $cgu_num: $human"
            total_cgu=$((total_cgu + size))
        done

        echo ""
        echo "Total CGU size: $(echo "$total_cgu" | awk '{printf "%.0fMB", $1/1048576}')"
    fi
fi
