# `sov-sequencer-registry` module

The `sov-sequencer-registry` module is responsible for sequencer registration, slashing, and rewards. At the moment, only a centralized sequencer is supported. The sequencer's address and bond are registered during the rollup deployment.

### The `sov-sequencer-registry` module offers the following functionality:

Hooks:

The `sov-sequencer-registry` module does not expose any call messages, and rollup users cannot directly modify the state of the sequencer. Instead, the module implements `ApplyBlobHooks` trait. 
