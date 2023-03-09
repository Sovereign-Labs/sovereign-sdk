use crate::storage::{StorageKey, StorageValue};
use first_read_last_write_cache::{
    cache::{self, CacheLog, FirstReads},
    CacheKey, CacheValue, MergeError,
};

/// `ValueReader` Reads a value from an external data source.
pub trait ValueReader {
    fn read_value(&self, key: StorageKey) -> Option<StorageValue>;
}

/// Caches reads and writes for a (key, value) pair. On the first read the value is fetched
/// from an external source represented by the `ValueReader` trait. On following reads,
/// the cache checks if the value we read was inserted before.
#[derive(Default)]
pub(crate) struct StorageInternalCache {
    slot_cache: CacheLog,
    tx_cache: CacheLog,
}

impl StorageInternalCache {
    /// Gets a value from the cache or reads it from the provided `ValueReader`.
    pub(crate) fn get_or_fetch<VR: ValueReader>(
        &mut self,
        key: StorageKey,
        value_reader: &VR,
    ) -> Option<StorageValue> {
        let cache_key = key.clone().as_cache_key();
        let cache_value = self.get_value_from_cache(cache_key.clone());

        match cache_value {
            cache::ValueExists::Yes(cache_value_exists) => {
                self.add_read(cache_key, cache_value_exists.clone());
                cache_value_exists.map(StorageValue::new_from_cache_value)
            }
            // If the value does not exist in the cache, then fetch it from an external source.
            cache::ValueExists::No => {
                let storage_value = value_reader.read_value(key);
                let cache_value = storage_value.as_ref().map(|v| v.clone().as_cache_value());

                self.add_read(cache_key, cache_value);
                storage_value
            }
        }
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

    pub(crate) fn merge(&mut self) -> Result<(), MergeError> {
        self.slot_cache.merge_left(&mut self.tx_cache)
    }

    pub(crate) fn merge_reads_and_discard_writes(&mut self) -> Result<(), MergeError> {
        self.slot_cache.merge_reads_left(&mut self.tx_cache)
    }

    pub(crate) fn slot_cache(&mut self) -> &mut CacheLog {
        &mut self.slot_cache
    }

    pub(crate) fn get_first_reads(&self) -> FirstReads {
        self.slot_cache.get_first_reads()
    }

    fn get_value_from_cache(&self, cache_key: CacheKey) -> cache::ValueExists {
        let cache_value = self.tx_cache.get_value(&cache_key);

        match cache_value {
            exists @ cache::ValueExists::Yes(_) => exists,
            // If the value does not exist in the tx cache, then fetch it from the slot cache.
            cache::ValueExists::No => self.slot_cache.get_value(&cache_key),
        }
    }

    fn add_read(&mut self, key: CacheKey, value: Option<CacheValue>) {
        self.tx_cache
            .add_read(key, value)
            // It is ok to panic here, we must guarantee that the cache is consistent.
            .unwrap_or_else(|e| panic!("Inconsistent read from the cache: {e:?}"));
    }
}
