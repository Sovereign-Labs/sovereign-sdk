# `sov-state`

This crate provides abstractions specifically designed for storing and retrieving data from a permanent storage, tailored to be used within the Module System.

The `sov-state` crate provides NOMT-backed implementations of the Storage trait: `NomtVerifierStorage` and `NomtProverStorage`. These implementations handle the storage and retrieval of data within the context of the `Zkp` and `Native` execution modes, respectively. They encapsulate the required logic and interactions with the storage system, allowing module developers to work with a consistent interface regardless of the execution mode.

### `CacheLog`:

Performing state updates and generating witnesses is a costly process. Thus, it is logical to incorporate caching layers to alleviate these issues. The `CacheLog` writes data to the in-memory map and reads from the backing store only if the data is absent from the map.
