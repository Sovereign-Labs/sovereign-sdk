#!/bin/bash
export SOV_BENCH_BLOCKS=10
export SOV_BENCH_TXNS_PER_BLOCKS=10000
export TPS=1000
(cd examples/demo-rollup/sov-benchmarks/src/node && make basic 2>&1) | tee output.log
(cd examples/demo-rollup/sov-benchmarks/src/node && make basic-nomt 2>&1) | tee nomt_output.log

check_tps() {
    local output_file=$1
    
    echo "Checking TPS for $output_file..."
    
    tps_line=$(grep -w "Transactions per sec (TPS)" $output_file)
    
    if [ -z "$tps_line" ]; then
        echo "The line containing 'TPS' was not found in $output_file."
        return 1
    else
        tps_count=$(echo "$tps_line" | awk -F '|' '{print $3}' | sed 's/,//g')
        result=$(awk -v val="$tps_count" -v threshold="$TPS" 'BEGIN {print (val < threshold) ? "FAIL" : "PASS"}')
        if [ "$result" = "FAIL" ]; then
            echo "The value for TPS in $output_file is less than $TPS. Failing the check. Value: $tps_count"
            return 1
        else
            echo "The value for TPS in $output_file is greater than $TPS. Passing the check. Value: $tps_count"
            return 0
        fi
    fi
}

check_tps "output.log" || exit 1
check_tps "nomt_output.log" || exit 1

exit 0
