mod jmt_storage;
mod map;
pub mod storage;

pub use map::StateMap;
pub use storage::Storage;

// A prefix prepended to each key before insertion and retrieval from the storage.
// All the collection types in this crate are backed by the same storage instance, this means that insertions of the same key
// to two different `StorageMaps` would collide with each other. We solve it by instantiating every collection type with a unique
// prefix that is prepended to each key.

#[derive(Debug)]
pub struct Prefix {}
