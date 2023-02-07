use std::sync::Arc;

// `Key` type for the `Storage`
#[derive(Clone, PartialEq, Eq)]
pub struct StorageKey {
    pub key: Arc<Vec<u8>>,
}

// `Value` type for the `Storage`
#[derive(Clone)]
pub struct StorageValue {
    pub value: Arc<Vec<u8>>,
}

// An interface for storing and retrieving values in the storage.
pub trait Storage {
    // Returns the value corresponding to the key or None if key is absent.
    fn get(&mut self, key: StorageKey, version: u64) -> Option<StorageValue>;

    // Inserts a key-value pair into the storage.
    fn set(&mut self, key: StorageKey, version: u64, value: StorageValue);

    // Deletes a key from the storage.
    fn delete(&mut self, key: StorageKey, version: u64);
}
