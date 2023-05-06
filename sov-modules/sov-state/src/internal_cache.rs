use crate::{
    storage::{StorageKey, StorageValue},
    Storage,
};
use first_read_last_write_cache::{
    cache::{self, CacheLog, ValueExists},
    CacheKey, CacheValue,
};

/// Caches reads and writes for a (key, value) pair. On the first read the value is fetched
/// from an external source represented by the `ValueReader` trait. On following reads,
/// the cache checks if the value we read was inserted before.
#[derive(Default)]
pub struct StorageInternalCache {
    pub tx_cache: CacheLog,
    pub ordered_db_reads: Vec<(CacheKey, Option<CacheValue>)>,
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
        key: StorageKey,
        value_reader: &S,
        witness: &S::Witness,
    ) -> Option<StorageValue> {
        let cache_key = key.clone().as_cache_key();
        let cache_value = self.get_value_from_cache(cache_key.clone());

        match cache_value {
            cache::ValueExists::Yes(cache_value_exists) => {
                // self.add_read(cache_key, cache_value_exists.clone());
                println!("Found value in inner cache");
                cache_value_exists.map(StorageValue::new_from_cache_value)
            }
            // If the value does not exist in the cache, then fetch it from an external source.
            cache::ValueExists::No => {
                println!("Value not found in inner cache. Fetching from storage.");
                let storage_value = value_reader.get(key, witness);
                let cache_value = storage_value.as_ref().map(|v| v.clone().as_cache_value());

                self.add_read(cache_key, cache_value);
                storage_value
            }
        }
    }

    pub fn try_get(&self, key: StorageKey) -> ValueExists {
        let cache_key = key.as_cache_key();
        self.get_value_from_cache(cache_key)
    }

    pub(crate) fn set(&mut self, key: StorageKey, value: StorageValue) {
        let cache_key = key.as_cache_key();
        let cache_value = value.as_cache_value();
        self.tx_cache.add_write(cache_key, Some(cache_value));
    }

    pub(crate) fn delete(&mut self, key: StorageKey) {
        let cache_key = key.as_cache_key();
        self.tx_cache.add_write(cache_key, None);
    }

    fn get_value_from_cache(&self, cache_key: CacheKey) -> cache::ValueExists {
        self.tx_cache.get_value(&cache_key)
    }

    pub fn merge_left(&mut self, rhs: Self) -> Result<(), first_read_last_write_cache::MergeError> {
        self.tx_cache.merge_left(rhs.tx_cache)
    }

    pub fn merge_reads_left(
        &mut self,
        rhs: Self,
    ) -> Result<(), first_read_last_write_cache::MergeError> {
        self.tx_cache.merge_reads_left(rhs.tx_cache)
    }

    pub fn merge_writes_left(
        &mut self,
        rhs: Self,
    ) -> Result<(), first_read_last_write_cache::MergeError> {
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
