# sov-app-template

This crate contains a generic `StateTransitionFunction` implementation (AppTemplate) that is suited to work with Sovereign `module-system`. The `AppTemplate` depends on set of traits   which assembled together define the rollup state transition logic.

1. The `DispatchCall` trait decodes serialized message and forwards it to a dedicated module.
1. The `Genesis` trait is responisble for the rollup initialization
1. The `TxVerifier` trait is responsble for transaction validation.
1. The `TxHooks` trait allows injecting custom logic into a transaction processing pipeline.

The `DispatchCall` and `Genesis` traits can be auto derived for any set of `modules` in the following way:

`#[derive(Genesis, DispatchCall, DispatchQuery, MessageCodec)]
#[serialization(borsh::BorshDeserialize, borsh::BorshSerialize)]
pub struct Runtime<C: Context> {
    sequencer: sequencer::Sequencer<C>,    
    bank: bank::Bank<C>,
    accounts: accounts::Accounts<C>,
    ...
    some other modules
}`

`Runtime` the entry point where all the rollup `modules` are assembled together.

The macros will generate implementation for `Genesis, DispatchCall, DispatchQuery` traits from the `sov-module-api` together with some plumbing code.


