use std::sync::Arc;

use sovereign_sdk::serial::Encode;

use crate::Prefix;

// `Key` type for the `Storage`
#[derive(Clone, PartialEq, Eq)]
pub struct StorageKey {
    key: Arc<Vec<u8>>,
}

impl StorageKey {
    pub fn key(&self) -> Arc<Vec<u8>> {
        self.key.clone()
    }
}

impl AsRef<Vec<u8>> for StorageKey {
    fn as_ref(&self) -> &Vec<u8> {
        &self.key
    }
}

impl StorageKey {
    pub fn new<K: Encode>(prefix: &Prefix, key: K) -> Self {
        let mut encoded_key = Vec::default();
        key.encode(&mut encoded_key);

        let mut full_key = Vec::<u8>::default();
        full_key.extend(prefix.as_bytes());
        full_key.extend(encoded_key);

        Self {
            key: Arc::new(full_key),
        }
    }
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
