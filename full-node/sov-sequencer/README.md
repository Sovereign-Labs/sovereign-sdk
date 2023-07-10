# Sov-Sequencer

Simple implementation of based sequencer generic over batch builder and DA service.

Exposes 2 RPC methods:


1. `sequencer_acceptTx` where input is suppose to be signed and serialized transaction. This transaction is stored in mempool
2. `sequencer_publishBatch` without any input, which builds the batch using batch builder and publishes it on DA layer.

## How to use it with `sov-cli`

`sov-cli` from `demo-stf` crate has support for interacting with sov-sequencer.


Make sure that this tool is build

```bash
cd examples/demo-stf
cargo build --bin sov-cli
```

### Submit transactions

This command is similar to serialize call from [`demo-rollup` README](../../examples/demo-rollup/README.md), with one last parameter, RPC endpoint for sov-sequencer.

When demo-rollup with enabled sequencer starts, it prints on which endpoint it listens:

```
2023-07-07T14:53:02.280562Z  INFO sov_demo_rollup: Starting RPC server at 127.0.0.1:12345
```


Let's submit 3 transactions: `create token`, `mint` and `transfer`:

```bash
# create token
./target/debug/sov-cli submit-call examples/demo-stf/src/sov-cli/test_data/token_deployer_private_key.json Bank examples/demo-stf/src/sov-cli/test_data/create_token.json 0 http://127.0.0.1:12345

# mint
./target/debug/sov-cli submit-call examples/demo-stf/src/sov-cli/test_data/minter_private_key.json Bank examples/demo-stf/src/sov-cli/test_data/mint.json 0 http://127.0.0.1:12345

# transfer
./target/debug/sov-cli submit-call examples/demo-stf/src/sov-cli/test_data/minter_private_key.json Bank examples/demo-stf/src/sov-cli/test_data/transfer.json 1 http://127.0.0.1:12345

```

Now these transactions are in the mempool

### Publish blob

In order to submit transactions to DA layer, sequencer needs to publish them. This can be done by triggering `publishBatch` endpooint:

```bash
./target/debug/sov-cli publish-batch http://127.0.0.1:12345
```

After some time, processed transaction should appear in logs of running rollup