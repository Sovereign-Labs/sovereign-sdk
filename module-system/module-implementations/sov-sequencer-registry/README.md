# The `sov-sequencer-registry` module.

The `sov-sequencer-registry` module is responsible for sequencer registration, slashing, and rewards. At the moment, only centralized sequencer is supported. The sequencer's address and bond are registered during the rollup deployment.

### The `sov-sequencer-registry` module offers the following functionality:

Hooks:

The `sov-sequencer-registry` module does not expose any call messages, and rollup users cannot directly modify the state of the sequencer. Instead, the module provides hooks that can be inserted at various points in the logic of the rollup's `state transition function`. The module supports the following hooks:

1. `lock`: Locks the sequencer bond.
1. `next_sequencer`: Since only centralized sequencer is supported currently, this hook always returns the same value, which is the registered sequencer address.
1. `reward`: Unlocks the sequencer bond, possibly with an additional tip.

If a sequencer misbehaves, the `reward` hook is never called, and the bond remains locked indefinitely.
