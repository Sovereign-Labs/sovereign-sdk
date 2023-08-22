#!/usr/bin/env bash

RPC_ENDPOINT="http://127.0.0.1:12345"
PRIVATE_KEY="../test-data/keys/token_deployer_private_key.json"
SOV_CLI="../../target/debug/sov-cli"

echo "Preparing..."
$SOV_CLI submit-transaction "$PRIVATE_KEY" Bank ../test-data/requests/create_token.json 0 "$RPC_ENDPOINT"
$SOV_CLI submit-transaction "$PRIVATE_KEY" SequencerRegistry ../test-data/requests/register_sequencer.json 1 "$RPC_ENDPOINT"
$SOV_CLI publish-batch "$RPC_ENDPOINT"


sleep 1
echo "Starting submitting transfers"
for nonce in {2..30}; do
  echo "Submitting transaction with nonce $nonce"
    $SOV_CLI submit-transaction "$PRIVATE_KEY" Bank ../test-data/requests/transfer.json "$nonce" "$RPC_ENDPOINT"
    if [ $((nonce % 3)) -eq 0 ]; then
        $SOV_CLI publish-batch "$RPC_ENDPOINT"
    fi
done