use std::sync::Arc;

// `Key` type for the `Storage`
#[derive(Clone, PartialEq, Eq)]
pub struct StorageKey {
    pub key: Arc<Vec<u8>>,
}

impl From<&'static str> for StorageKey {
    fn from(value: &'static str) -> Self {
        Self {
            key: Arc::new(value.as_bytes().to_vec()),
        }
    }
}

// `Value` type for the `Storage`
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StorageValue {
    pub value: Arc<Vec<u8>>,
}

impl From<&'static str> for StorageValue {
    fn from(value: &'static str) -> Self {
        Self {
            value: Arc::new(value.as_bytes().to_vec()),
        }
    }
}

// An interface for storing and retrieving values in the storage.
pub trait Storage {
    // Returns the value corresponding to the key or None if key is absent.
    fn get(&self, key: StorageKey) -> Option<StorageValue>;

    // Inserts a key-value pair into the storage.
    fn set(&self, key: StorageKey, value: StorageValue);

    // Deletes a key from the storage.
    fn delete(&self, key: StorageKey);
}
