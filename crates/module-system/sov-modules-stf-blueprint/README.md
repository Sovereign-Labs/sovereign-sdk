# `sov-default-stf`

### `StfBlueprint`

This crate contains an implementation of a `StateTransitionFunction` called `StfBlueprint` that is specifically designed to work with the Module System. The `StfBlueprint` relies on a set of traits that, when combined, define the logic for transitioning the rollup state.

1. The `DispatchCall` trait is responsible for decoding serialized messages and forwarding them to the appropriate module.
1. The `Genesis` trait handles the initialization process of the rollup. It sets up the initial state upon the rollup deployment.
1. The `TxHooks` trait allows for the injection of custom logic into the transaction processing pipeline. It provides a mechanism to execute additional actions or perform specific operations during the transaction processing phase.

### `Runtime`

Both the `DispatchCall` and `Genesis` traits can be automatically derived (see `RT` in the above snippet) for any set of modules:

```rust ignore
#[derive(Genesis, DispatchCall, MessageCodec)]
pub struct Runtime<S: Spec> {
    accounts: accounts::Accounts<S>,
    bank: sov_bank::Bank<S>,
    sequencer: sequencer::Sequencer<S>,
    ...
    some other modules
}
```

The `Runtime` struct acts as the entry point where all the rollup modules are assembled together. The `#[derive]` macro generates the necessary implementations for the `Genesis and DispatchCall` traits from the `sov-module-api` crate.

To obtain an instance of the `StateTransitionFunction`, you can pass a`Runtime`, to the `StfBlueprint::new(..)` method.
