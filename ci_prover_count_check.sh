#!/bin/bash
export BLOCKS=1
export TXNS_PER_BLOCK=10
export NUM_PUB_KEYS=100
export CYCLE_COUNT=1000000
cargo bench --bench prover_bench --features bench 2>&1 | tee output.log

output_file="output.log"

verify_line=$(grep -w "verify" $output_file)

if [ -z "$verify_line" ]; then
    echo "The line containing 'verify' was not found."
    exit 1
else
    average_cycles=$(echo $verify_line | awk '{print $4}' | sed 's/,//g') # Remove commas if present

    if [ -n "$average_cycles" ] && [ "$average_cycles" -lt $CYCLE_COUNT ]; then
        echo "The value for 'verify' is less than $CYCLE_COUNT. Passing the check. Value: $average_cycles"
        exit 0
    elif [ -n "$average_cycles" ]; then
        echo "The value for 'verify' is greater than $CYCLE_COUNT. Failing the check. Value: $average_cycles"
        exit 0
    else
        echo "Unable to extract the 'Average Cycles' value."
        exit 1
    fi
fi
exit 1