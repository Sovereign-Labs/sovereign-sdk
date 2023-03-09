use std::{fmt::Display, sync::Arc};

use crate::{utils::AlignedVec, Prefix};
use first_read_last_write_cache::{CacheKey, CacheValue};
use hex;
use sovereign_sdk::serial::Encode;

// `Key` type for the `Storage`
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct StorageKey {
    key: Arc<Vec<u8>>,
}

impl StorageKey {
    pub fn key(&self) -> Arc<Vec<u8>> {
        self.key.clone()
    }

    pub fn as_cache_key(self) -> CacheKey {
        CacheKey { key: self.key }
    }
}

impl AsRef<Vec<u8>> for StorageKey {
    fn as_ref(&self) -> &Vec<u8> {
        &self.key
    }
}

impl Display for StorageKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:x?}", hex::encode(self.key().as_ref()))
    }
}

impl StorageKey {
    /// Creates a new StorageKey that combines a prefix and a key.
    pub fn new<K: Encode>(prefix: &Prefix, key: &K) -> Self {
        let mut encoded_key = Vec::default();
        key.encode(&mut encoded_key);

        let encoded_key = AlignedVec::new(encoded_key);

        let full_key = Vec::<u8>::with_capacity(prefix.len() + encoded_key.len());
        let mut full_key = AlignedVec::new(full_key);
        full_key.extend(prefix.as_aligned_vec());
        full_key.extend(&encoded_key);

        Self {
            key: Arc::new(full_key.into_inner()),
        }
    }
}

// `Value` type for the `Storage`
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StorageValue {
    value: Arc<Vec<u8>>,
}

impl StorageValue {
    pub fn new<V: Encode>(value: V) -> Self {
        let mut encoded_value = Vec::default();
        value.encode(&mut encoded_value);
        Self {
            value: Arc::new(encoded_value),
        }
    }

    pub fn value(&self) -> &[u8] {
        &self.value
    }

    pub fn as_cache_value(self) -> CacheValue {
        CacheValue { value: self.value }
    }

    pub fn new_from_cache_value(cache_value: CacheValue) -> Self {
        Self {
            value: cache_value.value,
        }
    }

    pub fn new_from_bytes(value: Vec<u8>) -> Self {
        Self {
            value: Arc::new(value),
        }
    }
}

/// An interface for storing and retrieving values in the storage.
pub trait Storage {
    /// Returns the value corresponding to the key or None if key is absent.
    fn get(&self, key: StorageKey) -> Option<StorageValue>;

    /// Inserts a key-value pair into the storage.
    fn set(&mut self, key: StorageKey, value: StorageValue);

    /// Deletes a key from the storage.
    fn delete(&mut self, key: StorageKey);

    /// Merges the batch level and tx level cache.
    fn merge(&mut self);

    /// Merges the batch level and tx level cache, discarding any writes from tx level cache.
    fn merge_reads_and_discard_writes(&mut self);

    /// Saves modified values in the db and clears internal caches.
    fn finalize(&mut self) -> [u8; 32];
}

// Used only in tests.
#[cfg(test)]
impl From<&'static str> for StorageKey {
    fn from(key: &'static str) -> Self {
        Self {
            key: Arc::new(key.as_bytes().to_vec()),
        }
    }
}

// Used only in tests.
#[cfg(test)]
impl From<&'static str> for StorageValue {
    fn from(value: &'static str) -> Self {
        Self {
            value: Arc::new(value.as_bytes().to_vec()),
        }
    }
}
