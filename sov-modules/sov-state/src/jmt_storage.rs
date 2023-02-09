use std::{cell::RefCell, rc::Rc};

use crate::storage::{Storage, StorageKey, StorageValue};
use first_read_last_write_cache::{
    cache::{self, CacheLog},
    CacheValue,
};
use jellyfish_merkle_generic::Version;

// Storage backed by JMT.
#[derive(Default, Clone)]
pub struct JmtStorage {
    // Caches first read and last write for a particular key.
    cache: Rc<RefCell<CacheLog>>,
    _version: Version,
}

impl Storage for JmtStorage {
    fn get(&self, key: StorageKey) -> Option<StorageValue> {
        let cache_key = key.into();
        let cache_value = self.cache.borrow().get_value(&cache_key);

        match cache_value {
            cache::ExistsInCache::Yes(cache_value_exists) => match cache_value_exists.value.clone()
            {
                Some(value) => {
                    self.cache
                        .borrow_mut()
                        .add_read(cache_key, cache_value_exists)
                        // It is ok to panic here, we must guarantee that the cache is consistent.
                        .unwrap_or_else(|e| panic!("Inconsistent read from the cache: {e:?}"));

                    Some(StorageValue { value })
                }
                None => None,
            },
            // TODO If value does not exist in the cache, then fetch it from the JMT
            cache::ExistsInCache::No => todo!(),
        }
    }

    fn set(&mut self, key: StorageKey, value: StorageValue) {
        let cache_key = key.into();
        let cache_value = value.into();
        self.cache.borrow_mut().add_write(cache_key, cache_value);
    }

    fn delete(&mut self, key: StorageKey) {
        let cache_key = key.into();
        self.cache
            .borrow_mut()
            .add_write(cache_key, CacheValue::empty());
    }
}
