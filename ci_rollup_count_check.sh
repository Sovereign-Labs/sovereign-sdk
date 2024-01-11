#!/bin/bash
export BLOCKS=10
export TXNS_PER_BLOCK=10000
export TPS=1000
(cd examples/demo-rollup/benches/node && make basic 2>&1) | tee output.log

output_file="output.log"

verify_line=$(grep -w "Transactions per sec (TPS)" $output_file)

if [ -z "$verify_line" ]; then
    echo "The line containing 'verify' was not found."
    exit 1
else
    tps_count=$(echo $verify_line | awk -F '|' '{print $3}' | sed 's/,//g')
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