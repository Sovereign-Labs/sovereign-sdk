# `sov-sequencer-registry` module

The `sov-sequencer-registry` module manages sequencer registration, stake, slashing, and DA address rotation.

## Activating DA address rotation

`UpdateDaAddress` is accepted from rollup execution height `SEQUENCER_DA_ROTATION_ACTIVATION_HEIGHT` onward. Below it, every registry call keeps its previous state and gas behaviour, so historical replay is unchanged. Fresh chains set it to `0`. Existing chains pick a future height and ship it as a hard fork: every node and verifier must run the new binary before that height is reached.

Registrations made before activation are tracked lazily: the first `UpdateDaAddress` for such a sequencer starts tracking at the address being rotated. A rollup address that registered several DA addresses before activation keeps them and can rotate one at a time. After activation a rollup address may register only one DA address.

## Paymaster state is not rotated

`sov-paymaster` keys its sequencer-to-payer mapping and each payer's `authorized_sequencers` list by DA address, and `UpdateDaAddress` does not touch either. After a rotation the sequencer's batches are no longer sponsored until the payer (or one of its authorized updaters) sends `UpdatePolicy` replacing the old DA address with the new one. If that transaction is not included in a batch produced by the new DA address, the sequencer must also include `SetPayerForSequencer` in one of its batches.
