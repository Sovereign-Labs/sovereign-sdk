# `sov-state`

This crate provides abstractions specifically designed for storing and retrieving data from a permanent storage, tailored to be used within the Module System.

The `sov-state` crate provides two implementations of the Storage trait: `ZkStorage` and `ProverStorage`. These implementations handle the storage and retrieval of data within the context of the `Zkp` and `Native` execution modes, respectively. (To improve performance when zk-proof generation is not a concern, an additional implementation can be added that excludes the generation of the witness). These implementations encapsulate the required logic and interactions with the storage system, allowing module developers to work with a consistent interface regardless of the execution mode.

### `CacheLog`:

Performing state updates and generating witnesses is a costly process. Thus, it is logical to incorporate caching layers to alleviate these issues. The `CacheLog` writes data to the in-memory map and reads from the backing store only if the data is absent from the map.
