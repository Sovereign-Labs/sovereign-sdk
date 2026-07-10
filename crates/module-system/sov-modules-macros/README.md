# `sov-modules-macros`

This crate provides Rust macros specifically designed to be used with the Module System. When developing a module, the developer's primary focus should be on implementing the business logic, without having to worry about low-level details such as message serialization/deserialization or how messages are dispatched to the appropriate module.

To alleviate the burden of writing repetitive and mechanical code, this crate offers a collection of macros that generate the necessary boilerplate code.

The following derive macros are supported:

1. The `ModuleInfo`: Derives the `sov-modules-api::ModuleInfo` implementation for the underlying type.
1. The `Genesis`: Derives the `sov-modules-api::Genesis` implementation for the underlying type.
1. The `DispatchCall`: Derives the `sov-modules-api::DispatchCall` implementation for the underlying type.
1. The `MessageCodec`: Adds message serialization/deserialization functionality to the underlying type.

The definitions of the traits mentioned above can be found in the [sov-modules-api](../sov-modules-api/README.md) crate.

Example usage:

```rust

/// Runtime is a collection of sov modules defined in the rollup.
#[derive(Genesis, DispatchCall, MessageCodec)]
pub struct Runtime<S: Spec> {
    accounts: accounts::Accounts<S>,
    bank: sov_bank::Bank<S>,
    sequencer: sequencer::Sequencer<S>,
    ...
    some other modules
}

/// `Genesis` allow initialization of the rollup in following way:
runtime.genesis(&configuration, working_set)

/// `DispatchCall & MessageCodec` allows dispatching serialized messages to the appropriate module.
let call_result = RT::decode_call(message_data)

```

#### `constants.toml`

This crate enables the embedding of constants into the compiled binary, allowing runtime developers to set parameters such as gas prices without modifying the module code.

The root of this JSON file will include two default attributes: `gas_price` and `config`.

The `gas_price` attribute will specify the gas price charged by module execution.

The `config` attribute will act as a placeholder for runtime configuration.

Here is an example of a `constants.toml` file:

```toml
# Sovereign SDK constants

[gas.Bank]
create_token = [4, 4]
transfer = [5, 5]
burn = [2, 2]
mint = [2, 2]
freeze = [1, 1]

[constants]
DEFERRED_SLOTS_COUNT = 2
```

The default location of the `constants.toml` file is in the root directory of the current workspace. Nonetheless, this can be superseded by setting the environment variable `CONSTANTS_MANIFEST` during compilation.

The following command will assert a `/foo/bar/Cargo.toml` file exists, and will use `/foo/bar/constants.toml`.

```sh
CONSTANTS_MANIFEST=/foo/bar cargo build
```

The macro compilation will endeavor to obtain the workspace root of the current working directory. If the execution is taking place from an external location, such as `cargo build --manifest-path /foo/bar/Cargo.toml`, you should adjust the path accordingly.

```sh
CONSTANTS_MANIFEST=/foo/bar cargo build --manifest-path /foo/bar/Cargo.toml
```

#### `chain-metadata.toml`

The per-network chain metadata keys — `CHAIN_ID`, `CHAIN_NAME`, `CHAIN_HASH_OVERRIDES`, `BATCH_NAMESPACE`, and `PROOF_NAMESPACE` — can be split into a `chain-metadata.toml` file sitting next to `constants.toml` (`chain-metadata.testing.toml` next to `constants.testing.toml`). It uses the same `[constants]` format:

```toml
[constants]
CHAIN_ID = 4321
CHAIN_NAME = "TestChain"
CHAIN_HASH_OVERRIDES = []
BATCH_NAMESPACE = { byte_string = "sov-test-b" }
PROOF_NAMESPACE = { hex = "0x736f762d746573742d70" }
```

These five keys — and only these — are then read from (and dependency-tracked against) the chain-metadata file. Tracking is per file: editing chain metadata recompiles the crates whose macro expansions read any of the five keys (plus their dependents), and no longer touches crates that only read `constants.toml`. All other keys, including the `[gas]` tables and module discriminants, always stay in `constants.toml`.

If `chain-metadata.toml` does not exist, the chain keys are read from `constants.toml` as before — the split is opt-in. To migrate, move the five keys into the new file and **remove them from `constants.toml`** (the removal is what triggers the one-time recompilation that picks up the new file). Once the file exists, every chain-metadata key must be defined in it: a chain key missing from an existing `chain-metadata.toml` is a compile error, even if `constants.toml` still defines it.
