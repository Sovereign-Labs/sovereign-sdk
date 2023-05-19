# The `Sequencer` module.

The `Sequencer` module has the responsibility of handling sequencer registration, slashing, and rewards. At the moment, only centralized sequencer is supported. The sequencer's address and bond are registered during the rollup deployment.

### The `Sequencer` module offers the following functionality:

Hooks:
The `Sequencer` module does not expose any call messages, and rollup users cannot directly modify the sequencer's state. Instead, the module provides hooks that can be inserted at various points in the state transition function logic. The following hooks are supported:

1. `lock`: Locks the sequencer bonds.
1. next_sequencer: Since only centralized sequencer is supported currently, this hook always returns the same value, which is the registered sequencer address.
1. reward: Unlocks the sequencer bond, possibly with an additional tip.

If a sequencer misbehaves, the `reward` hook is never called, and the bond remains locked indefinitely.

Queries:
1. `QueryMessage::GetSequencerAddressAndBalance` query retrieves the sequencer's address and balance on the rollup.