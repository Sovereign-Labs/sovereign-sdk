# sov-modules-macros

This crate provides Rust macros specifically designed to be used with the Sovereign `module-system`. When developing a module, the developer's primary focus is on implementing the business logic, without having to worry about low-level details such as message serialization/deserialization or how messages are dispatched to the appropriate module.

To alleviate the burden of writing repetitive and mechanical code, this crate offers a collection of macros that generate the necessary boilerplate code.

The following derive macros are supported:

1. The `ModuleInfo`: Derives the `sov-modules-api::ModuleInfo` implementation for the underlying type.
1. The `Genesis`: Derives the `sov-modules-api::Genesis` implementation for the underlying type.
1. The `DispatchCall`: Derives the `sov-modules-api::DispatchCall` implementation for the underlying type.
1. The `MessageCodec`: Adds message serialization/deserialization functionality to the underlying type.

The definitions of the traits mentioned above can be found in the [sov-modules-api](../sov-modules-api/README.md) crate.

Example of usage:

```rust

/// Runtime is a collection of sov modules defined in the rollup.
#[derive(Genesis, DispatchCall, MessageCodec)]
pub struct Runtime<C: Context> {
    accounts: accounts::Accounts<C>,
    bank: bank::Bank<C>,
    sequencer: sequencer::Sequencer<C>,
    ...
    some other modules
}

/// `Genesis` allow initialization of the rollup in following way:
runtime.genesis(&configuration, working_set)

/// `DispatchCall & MessageCodec` allows dispatching serialized messages to the appropriate module.
let call_result = RT::decode_call(message_data)

```
