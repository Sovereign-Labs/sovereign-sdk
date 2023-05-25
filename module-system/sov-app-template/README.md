# sov-app-template

### `AppTemplate`

This crate contains an implementation of a `StateTransitionFunction` called `AppTemplate`, specifically designed to work with the Sovereign `module-system`. The `AppTemplate` relies on a set of traits that, when combined, define the logic for transitioning the rollup state.

```rust
pub struct AppTemplate<C: Context, V, RT, H, Vm> {
    pub current_storage: C::Storage,
    pub runtime: RT,
    tx_verifier: V,
    tx_hooks: H,
    working_set: Option<WorkingSet<C::Storage>>,
    phantom_vm: PhantomData<Vm>,
}

impl<C: Context, V, RT, H, Vm> AppTemplate<C, V, RT, H, Vm>
where
    RT: DispatchCall<Context = C> + Genesis<Context = C>,
    V: TxVerifier,
    H: TxHooks<Context = C,..>,
{

    pub fn new(storage: C::Storage, runtime: RT, tx_verifier: V, tx_hooks: H) -> Self {
        ...
    }
  ...
}
```

1. The `DispatchCall` trait is responsible for decoding serialized messages and forwarding them to the appropriate module.
1. The `Genesis` trait handles the initialization process of the rollup. It sets up the initial state upon the rollup deployment.
1. The `TxVerifier` trait is responsible for validating transactions within the rollup. It ensures that incoming transactions meet the necessary criteria and are valid for execution.
1. The `TxHooks` trait allows for the injection of custom logic into the transaction processing pipeline. It provides a mechanism to execute additional actions or perform specific operations during the transaction processing phase.

### `Runtime`

Both the `DispatchCall` and `Genesis` traits can be automatically derived (see `RT` in the above snippet) for any set of modules:

```rust
#[derive(Genesis, DispatchCall, MessageCodec)]
#[serialization(borsh::BorshDeserialize, borsh::BorshSerialize)]
pub struct Runtime<C: Context> {
    accounts: accounts::Accounts<C>,
    bank: bank::Bank<C>,
    sequencer: sequencer::Sequencer<C>,
    ...
    some other modules
}
```

The `Runtime` struct acts as the entry point where all the rollup modules are assembled together. The `#[derive]` macro generates the necessary implementations for the `Genesis and DispatchCall` traits from the `sov-module-api` crate.

To obtain an instance of the `StateTransitionFunction`, you can pass a`Runtime`, along with implementations of the `TxVerifier` and `TxHooks` traits, to the `AppTemplate::new(..)` method. This ensures that the implementation of the `StateTransitionFunction` is straightforward and does not require manual integration or complex setup steps.
