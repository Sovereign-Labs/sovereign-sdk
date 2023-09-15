#!/usr/bin/env bash
trap 'jobs -p | xargs -r kill' EXIT
echo 'Running: '\''cd examples/demo-rollup/'\'''
cd examples/demo-rollup/
if [ $? -ne 0 ]; then
    echo "Expected exit code 0, got $?"
    exit 1
fi
echo 'Running: '\''make clean'\'''
make clean
if [ $? -ne 0 ]; then
    echo "Expected exit code 0, got $?"
    exit 1
fi
echo 'Running: '\''make start   # Make sure to run `make stop` when you'\''re done with this demo'\!''\'''
make start   # Make sure to run `make stop` when you're done with this demo!
if [ $? -ne 0 ]; then
    echo "Expected exit code 0, got $?"
    exit 1
fi
echo 'Running: '\''git status'\'''
git status
if [ $? -ne 0 ]; then
    echo "Expected exit code 0, got $?"
    exit 1
fi
echo 'Running: '\''cargo run'\'''
cargo run &
sleep 20
echo 'Running: '\''make test-create-token'\'''
make test-create-token
if [ $? -ne 0 ]; then
    echo "Expected exit code 0, got $?"
    exit 1
fi
echo 'Running: '\''cargo run --bin sov-cli -- --help'\'''

output=$(cargo run --bin sov-cli -- --help)
expected='# Make sure you'\''re still in `examples/demo-rollup`
Usage: sov-cli <COMMAND>

Commands:
  transactions  Generate, sign, and send transactions
  keys          View and manage keys associated with this wallet
  rpc           Query the current state of the rollup and send transactions
  help          Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version
'
# Either of the two must be a substring of the other. This kinda protects us
# against whitespace differences, trimming, etc.
if ! [[ $output == *"$expected"* || $expected == *"$output"* ]]; then
    echo "'$expected' not found in text:"
    echo "'$output'"
    exit 1
fi

if [ $? -ne 0 ]; then
    echo "Expected exit code 0, got $?"
    exit 1
fi
echo 'Running: '\''cargo run --bin sov-cli -- transactions import from-file -h'\'''

output=$(cargo run --bin sov-cli -- transactions import from-file -h)
expected='Import a transaction from a JSON file at the provided path

Usage: sov-cli transactions import from-file <COMMAND>

Commands:
  bank                A subcommand for the `bank` module
  sequencer-registry  A subcommand for the `sequencer_registry` module
  value-setter        A subcommand for the `value_setter` module
  accounts            A subcommand for the `accounts` module
  help                Print this message or the help of the given subcommand(s)

Options:
  -h, --help  Print help
'
# Either of the two must be a substring of the other. This kinda protects us
# against whitespace differences, trimming, etc.
if ! [[ $output == *"$expected"* || $expected == *"$output"* ]]; then
    echo "'$expected' not found in text:"
    echo "'$output'"
    exit 1
fi

if [ $? -ne 0 ]; then
    echo "Expected exit code 0, got $?"
    exit 1
fi
echo 'Running: '\''cargo run --bin sov-cli -- transactions import from-file bank --path ../test-data/requests/transfer.json'\'''

output=$(cargo run --bin sov-cli -- transactions import from-file bank --path ../test-data/requests/transfer.json)
expected='Adding the following transaction to batch:
{
  "bank": {
    "Transfer": {
      "to": "sov1l6n2cku82yfqld30lanm2nfw43n2auc8clw7r5u5m6s7p8jrm4zqrr8r94",
      "coins": {
        "amount": 200,
        "token_address": "sov1zdwj8thgev2u3yyrrlekmvtsz4av4tp3m7dm5mx5peejnesga27svq9m72"
      }
    }
  }
}
'
# Either of the two must be a substring of the other. This kinda protects us
# against whitespace differences, trimming, etc.
if ! [[ $output == *"$expected"* || $expected == *"$output"* ]]; then
    echo "'$expected' not found in text:"
    echo "'$output'"
    exit 1
fi

if [ $? -ne 0 ]; then
    echo "Expected exit code 0, got $?"
    exit 1
fi
echo 'Running: '\''cargo run --bin sov-cli rpc submit-batch by-address sov1l6n2cku82yfqld30lanm2nfw43n2auc8clw7r5u5m6s7p8jrm4zqrr8r94'\'''
cargo run --bin sov-cli rpc submit-batch by-address sov1l6n2cku82yfqld30lanm2nfw43n2auc8clw7r5u5m6s7p8jrm4zqrr8r94
if [ $? -ne 0 ]; then
    echo "Expected exit code 0, got $?"
    exit 1
fi
echo 'Running: '\''curl -X POST -H "Content-Type: application/json" -d '\''{"jsonrpc":"2.0","method":"bank_supplyOf","params":["sov1zdwj8thgev2u3yyrrlekmvtsz4av4tp3m7dm5mx5peejnesga27svq9m72"],"id":1}'\'' http://127.0.0.1:12345'\'''

output=$(curl -X POST -H "Content-Type: application/json" -d '{"jsonrpc":"2.0","method":"bank_supplyOf","params":["sov1zdwj8thgev2u3yyrrlekmvtsz4av4tp3m7dm5mx5peejnesga27svq9m72"],"id":1}' http://127.0.0.1:12345)
expected='{"jsonrpc":"2.0","result":{"amount":1000},"id":1}
'
# Either of the two must be a substring of the other. This kinda protects us
# against whitespace differences, trimming, etc.
if ! [[ $output == *"$expected"* || $expected == *"$output"* ]]; then
    echo "'$expected' not found in text:"
    echo "'$output'"
    exit 1
fi

if [ $? -ne 0 ]; then
    echo "Expected exit code 0, got $?"
    exit 1
fi
echo "All tests passed!"; exit 0
