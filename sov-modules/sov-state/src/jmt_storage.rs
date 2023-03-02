use crate::{
    internal_cache::ValueReader,
    storage::{GenericStorage, StorageKey, StorageValue},
};
use first_read_last_write_cache::cache::{CacheLog, FirstReads};
use sovereign_db::state_db::StateDB;

impl ValueReader for StateDB {
    fn read_value(&self, key: StorageKey) -> Option<StorageValue> {
        match self.get_value_option_by_key(0, key.as_ref()) {
            Ok(value) => value.map(StorageValue::new_from_bytes),
            // It is ok to panic here, we assume the db is available and consistent.
            Err(e) => panic!("Unable to read value from db: {e}"),
        }
    }
}

pub type JmtStorage = GenericStorage<StateDB>;

impl JmtStorage {
    #[cfg(any(test, feature = "temp"))]
    pub fn temporary() -> Self {
        Self::new(StateDB::temporary())
    }

    /// Gets the first reads from the JmtStorage.
    pub fn get_first_reads(&self) -> FirstReads {
        let cache: &CacheLog = &self.internal_cache.cache.borrow();
        cache.get_first_reads()
    }
}
