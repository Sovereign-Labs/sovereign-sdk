use std::fmt::Display;
use std::sync::Arc;

use borsh::{BorshDeserialize, BorshSerialize};
use hex;
use serde::{Deserialize, Serialize};
use sov_first_read_last_write_cache::{CacheKey, CacheValue};

use crate::internal_cache::OrderedReadsAndWrites;
use crate::utils::AlignedVec;
use crate::witness::Witness;
use crate::Prefix;

// `Key` type for the `Storage`
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct StorageKey {
    key: Arc<Vec<u8>>,
}

impl From<CacheKey> for StorageKey {
    fn from(cache_key: CacheKey) -> Self {
        Self { key: cache_key.key }
    }
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
    pub fn new<K: BorshSerialize>(prefix: &Prefix, key: &K) -> Self {
        let encoded_key = key.try_to_vec().unwrap();
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
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct StorageValue {
    value: Arc<Vec<u8>>,
}

impl StorageValue {
    pub fn new<V: BorshSerialize>(value: &V) -> Self {
        let encoded_value = value.try_to_vec().unwrap();
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

    /// Indicates if storage is empty or not.
    /// Useful during initialization
    fn is_empty(&self) -> bool;
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
