# Sov-Sequencer

Simple implementation of based sequencer generic over batch builder and DA service.

Exposes 2 RPC methods:


1. `sequencer_acceptTx` where input is suppose to be signed and serialized transaction. This transaction is stored in mempool
2. `sequencer_publishBatch` without any input, which builds the batch using batch builder and publishes it on DA layer.
