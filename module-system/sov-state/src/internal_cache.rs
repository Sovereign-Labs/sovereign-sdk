use sov_first_read_last_write_cache::cache::{self, CacheLog, ValueExists};
use sov_first_read_last_write_cache::{CacheKey, CacheValue};

use crate::storage::{StorageKey, StorageValue};
use crate::Storage;

/// Caches reads and writes for a (key, value) pair. On the first read the value is fetched
/// from an external source represented by the `ValueReader` trait. On following reads,
/// the cache checks if the value we read was inserted before.
#[derive(Default)]
pub struct StorageInternalCache {
    pub tx_cache: CacheLog,
    pub ordered_db_reads: Vec<(CacheKey, Option<CacheValue>)>,
}

/// A struct that contains the values read from the DB and the values to be written, both in
/// deterministic order.
#[derive(Debug, Default)]
pub struct OrderedReadsAndWrites {
    pub ordered_reads: Vec<(CacheKey, Option<CacheValue>)>,
    pub ordered_writes: Vec<(CacheKey, Option<CacheValue>)>,
}

impl From<StorageInternalCache> for OrderedReadsAndWrites {
    fn from(val: StorageInternalCache) -> Self {
        let mut writes = val.tx_cache.take_writes();
        // TODO: Make this more efficient
        writes.sort_by(|(k1, _), (k2, _)| k1.cmp(k2));
        Self {
            ordered_reads: val.ordered_db_reads,
            ordered_writes: writes,
        }
    }
}

impl From<StorageInternalCache> for CacheLog {
    fn from(val: StorageInternalCache) -> Self {
        val.tx_cache
    }
}

impl StorageInternalCache {
    /// Gets a value from the cache or reads it from the provided `ValueReader`.
    pub(crate) fn get_or_fetch<S: Storage>(
        &mut self,
        key: &StorageKey,
        value_reader: &S,
        witness: &S::Witness,
    ) -> Option<StorageValue> {
        let cache_key = key.to_cache_key();
        let cache_value = self.get_value_from_cache(&cache_key);

        match cache_value {
            cache::ValueExists::Yes(cache_value_exists) => cache_value_exists.map(Into::into),
            // If the value does not exist in the cache, then fetch it from an external source.
            cache::ValueExists::No => {
                let storage_value = value_reader.get(key, witness);
                let cache_value = storage_value.as_ref().map(|v| v.clone().into_cache_value());

                self.add_read(cache_key, cache_value);
                storage_value
            }
        }
    }

    pub fn try_get(&self, key: &StorageKey) -> ValueExists {
        let cache_key = key.to_cache_key();
        self.get_value_from_cache(&cache_key)
    }

    pub(crate) fn set(&mut self, key: &StorageKey, value: StorageValue) {
        let cache_key = key.to_cache_key();
        let cache_value = value.into_cache_value();
        self.tx_cache.add_write(cache_key, Some(cache_value));
    }

    pub(crate) fn delete(&mut self, key: &StorageKey) {
        let cache_key = key.to_cache_key();
        self.tx_cache.add_write(cache_key, None);
    }

    fn get_value_from_cache(&self, cache_key: &CacheKey) -> cache::ValueExists {
        self.tx_cache.get_value(cache_key)
    }

    pub fn merge_left(
        &mut self,
        rhs: Self,
    ) -> Result<(), sov_first_read_last_write_cache::MergeError> {
        self.tx_cache.merge_left(rhs.tx_cache)
    }

    pub fn merge_reads_left(
        &mut self,
        rhs: Self,
    ) -> Result<(), sov_first_read_last_write_cache::MergeError> {
        self.tx_cache.merge_reads_left(rhs.tx_cache)
    }

    pub fn merge_writes_left(
        &mut self,
        rhs: Self,
    ) -> Result<(), sov_first_read_last_write_cache::MergeError> {
        self.tx_cache.merge_writes_left(rhs.tx_cache)
    }

    fn add_read(&mut self, key: CacheKey, value: Option<CacheValue>) {
        self.tx_cache
            .add_read(key.clone(), value.clone())
            // It is ok to panic here, we must guarantee that the cache is consistent.
            .unwrap_or_else(|e| panic!("Inconsistent read from the cache: {e:?}"));
        self.ordered_db_reads.push((key, value))
    }
}
