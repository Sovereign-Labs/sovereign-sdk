#!/usr/bin/env bash
trap 'trap - SIGTERM && kill -- -$$' SIGINT SIGTERM EXIT
cd examples/demo-rollup/
if [[ $? -ne 0 ]]; then
    exit 1
fi
make clean
if [[ $? -ne 0 ]]; then
    exit 1
fi
make start   # Make sure to run `make stop` when you're done with this demo!
if [[ $? -ne 0 ]]; then
    exit 1
fi
git status
if [[ $? -ne 0 ]]; then
    exit 1
fi
cargo run &
make test-create-token
if [[ $? -ne 0 ]]; then
    exit 1
fi
cargo run --bin sov-cli
if [[ $? -ne 0 ]]; then
    exit 1
fi
cargo run --bin sov-cli -- transactions import from-file -h
if [[ $? -ne 0 ]]; then
    exit 1
fi
cargo run --bin sov-cli -- transactions import from-file bank --path ../test-data/requests/transfer.json
if [[ $? -ne 0 ]]; then
    exit 1
fi
cargo run --bin sov-cli rpc submit-batch by-address sov1l6n2cku82yfqld30lanm2nfw43n2auc8clw7r5u5m6s7p8jrm4zqrr8r94
if [[ $? -ne 0 ]]; then
    exit 1
fi

                output=$(curl -X POST -H "Content-Type: application/json" -d '{"jsonrpc":"2.0","method":"bank_supplyOf","params":["sov1zdwj8thgev2u3yyrrlekmvtsz4av4tp3m7dm5mx5peejnesga27svq9m72"],"id":1}' http://127.0.0.1:12345)
                expected='{"jsonrpc":"2.0","result":{"amount":1000},"id":1}
'
                if [[ $output == *"$expected"* ]]; then
                    echo "'$expected' found"
                else
                    echo "'$expected' not found in text:"
                    echo "'$output'"
                    exit 1
                fi
                
if [[ $? -ne 0 ]]; then
    exit 1
fi

        echo "All tests passed!"
        exit 0
        
