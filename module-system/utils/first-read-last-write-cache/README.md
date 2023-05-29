# `first-read-last-write-cache`
This crate provides `first-read-last-write-cache` data structure specifically designed to be integrated with `sov-state::WorkingSet` used in the [Module System](../../README.md).

## Why `first-read-last-write-cache`?
Emulating a (Sparse) Merkle Tree inside a zero-knowledge computation is relatively inefficient because hashing tends to be an expensive operation in the zk context. For this reason, we want to minimize the number of MT accesses. For additional efficiency, we seek to "batch" accesses wherever possible. This allows us to share intermediate hash computations, reducing the total number of operations to be performed.

Rather than verifying/applying reads and writes immediately, we propose storing read/write values in a cache-like data structure for later batch verification. This structure (`CacheLog`) will store the first value read and the most-recent value written to each location. Assuming the correctness of the black-box VM implementation, these two pieces of information are sufficient for the verification of all state accesses and the construction of a (verified) post-state.


## Example
This is an implementation of a cache that tracks the first read and the last write for a particular key. The cache ensures consistency between reads and writes.

For example:

|key   	|operation 1|operation 2|operation 3|(first read, last write)   |
|:---   |:---	    |:---       |:---       |:---                       |
|k      |Read(1)    |Write(3)   |Read(3)    |((Read(1), Write(3))       |
|k      |Write(3)   |Read(3)    |Read(3)    |(_      , Write(3))        |
|k      |Write(5)   |Read(3)    |...        |inconsistent               |

## Usage:

```rust
    let mut cache = CacheLog::default();
    let value = match cache.get_value(&key) {
        ExistsInCache::Yes(value) => value,
        ExistsInCache::No => {
            // This is some "expensive" operation, for example a db lookup.
            let new_value = Some(Arc::new(vec![4, 5, 6, 7]));
            cache_log.add_read(key, new_value)?;
            new_value
        }
    };
```

