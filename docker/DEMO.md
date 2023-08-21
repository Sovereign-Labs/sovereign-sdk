

```bash
# Sent to node 2, should fail
target/debug/sov-cli submit-transaction examples/test-data/keys/token_deployer_private_key.json Bank examples/test-data/requests/create_token.json 0 http://127.0.0.1:12346
target/debug/sov-cli publish-batch http://127.0.0.1:12346

# Registering second sequencer
target/debug/sov-cli submit-transaction examples/test-data/keys/token_deployer_private_key.json SequencerRegistry examples/test-data/requests/register_sequencer.json 0 http://127.0.0.1:12345
target/debug/sov-cli publish-batch http://127.0.0.1:12345

# Try on second sequencer again
target/debug/sov-cli submit-transaction examples/test-data/keys/token_deployer_private_key.json Bank examples/test-data/requests/transfer.json 1 http://127.0.0.1:12346
target/debug/sov-cli publish-batch http://127.0.0.1:12346
```