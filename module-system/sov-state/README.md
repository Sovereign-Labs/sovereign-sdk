# `sov-state`

This crate provides abstractions specifically designed for storing and retrieving data from a permanent storage, tailored to be used within the Module System.

## High-level explanation

At a high level, the crate offers two main abstractions that module developers can utilize to access data:

1. `StateValue`: Is used to store a single value in the state. It provides methods to set a value and retrieve it later.
1. `StateMap`: Is used to store mappings in the state. It allows module developers to associate keys with values and retrieve them accordingly.

In the future, this crate aims to introduce additional abstractions, such as `StateVec`, to further enhance capabilities of data storage within the Module System.

Here is a snippet showcasing part of the `StateValue` API:

```Rust
impl StateValue<V> {

    /// Sets a value in the StateValue.
    pub fn set<S: Storage>(&self, value: V, working_set: &mut WorkingSet<S>) {
        // Implementation details
    }

    /// Gets a value from the StateValue or None if the value is absent.
    pub fn get<S: Storage>(&self, working_set: &mut WorkingSet<S>) -> Option<V> {
       // Implementation details
    }

    // Additional methods
    // ...
}
```

Both `get` and `set `methods require a `WorkingSet` parameter, which acts as a wrapper around a `key-value` store with additional caching layers.

Module developers can interact with the `WorkingSet`, `StateValue`, and `StateMap` without worrying about the inner workings of these components. Instead, they can treat them as black boxes that handle the storage and retrieval of data.

The above API is used in the following way:

```rust
state.value.set(&some_value, working_set);
let maybe_value = state.value.get(working_set);

```

## Low-level explanation

It's important to note that an understanding of this section is not necessarily required for efficient usage of the `sov-state`.

### `Native` & `Zkp` execution:

During `Native` execution, the data is stored in a `key-value` store, which is accessed through the `WorkingSet`. It's worth mentioning that the actual storage mechanism, such as `RocksDB`, is only accessible during this phase when the full node executes the transaction and updates the state.

In contrast, during the `Zkp` phase, when a cryptographic proof of correct execution is generated, the Module System doesn't have direct access to the underlying database. Instead, it relies on a "witness" produced during the `Native` execution. The system performs cryptographic checks, typically using variations of Merkle trees, to verify that the state was updated correctly. Despite the differences in access to the storage mechanism, both scenarios can be abstracted behind the same interface.

The `Storage` abstraction is defined as follows:

```Rust
pub trait Storage: Clone {
    type Witness: Witness;
    /// The runtime config for this storage instance.
    type RuntimeConfig;

    fn with_config(config: Self::RuntimeConfig) -> Result<Self, anyhow::Error>;

    /// Returns the value corresponding to the key or None if key is absent.
    fn get(&self, key: StorageKey, witness: &Self::Witness) -> Option<StorageValue>;

    /// Validate all of the storage accesses in a particular cache log,
    /// returning the new state root after applying all writes
    fn validate_and_commit(
        &self,
        state_accesses: OrderedReadsAndWrites,
        witness: &Self::Witness,
    ) -> Result<[u8; 32], anyhow::Error>;
}
```

The `sov-state` crate provides two implementations of the Storage trait: `ZkStorage` and `ProverStorage`. These implementations handle the storage and retrieval of data within the context of the `Zkp` and `Native` execution modes, respectively. (To improve performance when zk-proof generation is not a concern, an additional implementation can be added that excludes the generation of the witness). These implementations encapsulate the required logic and interactions with the storage system, allowing module developers to work with a consistent interface regardless of the execution mode.

### `WorkingSet`:

Performing state updates and generating witnesses is a costly process. Thus, it is logical to incorporate caching layers to alleviate these issues. The `WorkingSet` writes data to the in-memory map and reads from the backing store only if the data is absent from the map. For more information about our cache, refer to the [`sov-first-read-last-write-cache`](../utils/sov-first-read-last-write-cache) crate. Furthermore, caches simplify the process of implementing state reverts. In the event that a specific transaction needs to be reverted, we can simply discard all the writes made to the relevant cache.
