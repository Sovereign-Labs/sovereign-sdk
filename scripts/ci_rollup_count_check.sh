#!/bin/bash
export SOV_BENCH_BLOCKS=10
export SOV_BENCH_TXNS_PER_BLOCKS=10000
export TPS=1000
(cd examples/demo-rollup/sov-benchmarks/src/node && make basic 2>&1) | tee output.log

output_file="output.log"

tps_line=$(grep -w "Transactions per sec (TPS)" $output_file)

if [ -z "$tps_line" ]; then
    echo "The line containing 'TPS' was not found."
    exit 1
else
    tps_count=$(echo "$tps_line" | awk -F '|' '{print $3}' | sed 's/,//g')
    result=$(awk -v val="$tps_count" -v threshold="$TPS" 'BEGIN {print (val < threshold) ? "FAIL" : "PASS"}')
    if [ "$result" = "FAIL" ]; then
        echo "The value for TPS is less than $TPS. Failing the check. Value: $tps_count"
        exit 1
    else
        echo "The value for TPS is greater than $TPS. Passing the check. Value: $tps_count"
        exit 0
    fi
fi
exit 1
