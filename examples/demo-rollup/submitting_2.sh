#!/usr/bin/env bash

RPC_ENDPOINT="http://127.0.0.1:12346"
PRIVATE_KEY="../test-data/keys/minter_private_key.json"
SOV_CLI="../../target/debug/sov-cli"

echo "Starting !!!"

for nonce in {0..30}; do
  echo "Submitting transaction with nonce $nonce"
    $SOV_CLI submit-transaction "$PRIVATE_KEY" Bank ../test-data/requests/transfer.json "$nonce" "$RPC_ENDPOINT"
    if [ $((nonce % 3)) -eq 0 ]; then
        $SOV_CLI publish-batch "$RPC_ENDPOINT"
    fi
done