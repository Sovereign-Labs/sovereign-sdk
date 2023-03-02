use crate::{
    internal_cache::{StorageInternalCache, ValueReader},
    storage::{StorageKey, StorageValue},
    Storage,
};
use first_read_last_write_cache::cache::{CacheLog, FirstReads};

pub type JmtDb = sovereign_db::state_db::StateDB;

impl ValueReader for JmtDb {
    fn read_value(&self, key: StorageKey) -> Option<StorageValue> {
        match self.get_value_option_by_key(0, key.as_ref()) {
            Ok(value) => value.map(StorageValue::new_from_bytes),
            Err(e) => panic!("Unable to read value from db: {e}"),
        }
    }
}

/// Storage backed by JmtDb.
#[derive(Clone)]
pub struct JmtStorage {
    pub(crate) db: JmtDb,
    // Caches first read and last write for a particular key.
    pub(crate) internal_cache: StorageInternalCache,
}

impl JmtStorage {
    /// Creates a new JmtStorage.
    pub fn new(jmt: JmtDb) -> Self {
        Self {
            internal_cache: StorageInternalCache::default(),
            db: jmt,
        }
    }

    #[cfg(any(test, feature = "temp"))]
    pub fn temporary() -> Self {
        Self {
            internal_cache: StorageInternalCache::default(),
            db: JmtDb::temporary(),
        }
    }

    /// Gets the first reads from the JmtStorage.
    pub fn get_first_reads(&self) -> FirstReads {
        let cache: &CacheLog = &self.internal_cache.cache.borrow();
        cache.get_first_reads()
    }
}

impl Storage for JmtStorage {
    /// Instead of a config object, we just pass in the
    /// db instance directly for now.
    //  TODO: decide whether to use an actual config, or
    //  rename this type
    type Config = JmtDb;

    fn new(config: Self::Config) -> Self {
        Self {
            db: config,
            internal_cache: Default::default(),
        }
    }

    fn get(&self, key: StorageKey) -> Option<StorageValue> {
        self.internal_cache.get_or_fetch(key, &self.db)
    }

    fn set(&mut self, key: StorageKey, value: StorageValue) {
        self.internal_cache.set(key, value)
    }

    fn delete(&mut self, key: StorageKey) {
        self.internal_cache.delete(key)
    }
}
