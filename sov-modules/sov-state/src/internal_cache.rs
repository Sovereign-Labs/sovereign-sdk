use crate::storage::{StorageKey, StorageValue};
use first_read_last_write_cache::cache::{self, CacheLog};
use std::{cell::RefCell, rc::Rc};

/// `ValueReader` Reads a value from an external data source.
pub trait ValueReader {
    fn read_value(&self, key: StorageKey) -> Option<StorageValue>;
}

/// Caches reads and writes for a (key, value) pair. On the first read the value is fetched
/// from an external source represented by the `ValueReader` trait. On following reads,
/// the cache checks if the value we read was inserted before.
#[derive(Default, Clone)]
pub(crate) struct StorageInternalCache {
    pub(crate) cache: Rc<RefCell<CacheLog>>,
}

impl StorageInternalCache {
    /// Gets a value from the cache or reads it from the provided `ValueReader`.
    pub(crate) fn get_or_fetch<VR: ValueReader>(
        &self,
        key: StorageKey,
        value_reader: &VR,
    ) -> Option<StorageValue> {
        let cache_key = key.clone().as_cache_key();
        let cache_value = self.cache.borrow().get_value(&cache_key);

        match cache_value {
            cache::ValueExists::Yes(cache_value_exists) => {
                self.cache
                    .borrow_mut()
                    .add_read(cache_key, cache_value_exists.clone())
                    // It is ok to panic here, we must guarantee that the cache is consistent.
                    .unwrap_or_else(|e| panic!("Inconsistent read from the cache: {e:?}"));

                cache_value_exists.map(StorageValue::new_from_cache_value)
            }
            // If the value does not exist in the cache, then fetch it from an external source.
            cache::ValueExists::No => {
                let storage_value = value_reader.read_value(key);
                let cache_value = storage_value.as_ref().map(|v| v.clone().as_cache_value());

                self.cache
                    .borrow_mut()
                    .add_read(cache_key, cache_value)
                    .unwrap_or_else(|e| panic!("Inconsistent read from the cache: {e:?}"));

                storage_value
            }
        }
    }

    pub(crate) fn set(&mut self, key: StorageKey, value: StorageValue) {
        let cache_key = key.as_cache_key();
        let cache_value = value.as_cache_value();
        self.cache
            .borrow_mut()
            .add_write(cache_key, Some(cache_value));
    }

    pub(crate) fn delete(&mut self, key: StorageKey) {
        let cache_key = key.as_cache_key();
        self.cache.borrow_mut().add_write(cache_key, None);
    }
}
