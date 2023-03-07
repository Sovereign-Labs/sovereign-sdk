use std::{fs, path::Path, sync::Arc};

use crate::{
    internal_cache::{StorageInternalCache, ValueReader},
    storage::{StorageKey, StorageValue},
    Storage,
};
use first_read_last_write_cache::cache::FirstReads;
use jmt::KeyHash;
use sovereign_db::state_db::StateDB;
use sovereign_sdk::core::crypto;

impl ValueReader for StateDB {
    fn read_value(&self, key: StorageKey) -> Option<StorageValue> {
        match self.get_value_option_by_key(self.get_next_version(), key.as_ref()) {
            Ok(value) => value.map(StorageValue::new_from_bytes),
            // It is ok to panic here, we assume the db is available and consistent.
            Err(e) => panic!("Unable to read value from db: {e}"),
        }
    }
}

#[derive(Clone)]
pub struct JmtStorage {
    batch_cache: StorageInternalCache,
    tx_cache: StorageInternalCache,
    db: StateDB,
}

impl JmtStorage {
    #[cfg(any(test, feature = "temp"))]
    pub fn temporary() -> Self {
        let db = StateDB::temporary();
        Self::with_db(db).unwrap()
    }

    pub fn with_path(path: impl AsRef<Path>) -> Result<Self, anyhow::Error> {
        let db = StateDB::with_path(&path)?;
        Self::with_db(db)
    }

    fn with_db(db: StateDB) -> Result<Self, anyhow::Error> {
        Ok(Self {
            batch_cache: StorageInternalCache::default(),
            tx_cache: StorageInternalCache::default(),
            db,
        })
    }

    /// Gets the first reads from the JmtStorage.
    pub fn get_first_reads(&self) -> FirstReads {
        self.tx_cache.borrow().get_first_reads()
    }
}

impl Storage for JmtStorage {
    fn get(&self, key: StorageKey) -> Option<StorageValue> {
        self.tx_cache.get_or_fetch(key, &self.db)
    }

    fn set(&mut self, key: StorageKey, value: StorageValue) {
        self.tx_cache.set(key, value)
    }

    fn delete(&mut self, key: StorageKey) {
        self.tx_cache.delete(key)
    }

    fn merge(&mut self) {
        self.batch_cache
            .merge(&mut self.tx_cache)
            .unwrap_or_else(|e| panic!("Cache merge error: {e}"));
    }

    fn merge_reads_and_discard_writes(&mut self) {
        self.batch_cache
            .merge_reads_and_discard_writes(&mut self.tx_cache)
            .unwrap_or_else(|e| panic!("Cache merge error: {e}"));
    }

    fn finalize(&mut self) {
        let cache = &mut self.batch_cache.borrow_mut();

        let next_version = self.db.get_next_version();
        for (cache_key, cache_value) in cache.get_all_writes_and_clear_cache() {
            let key = Arc::try_unwrap(cache_key.key).unwrap_or_else(|arc| (*arc).clone());
            let key_hash = KeyHash(crypto::hash::sha2(key.as_ref()).0);

            let value =
                cache_value.map(|v| Arc::try_unwrap(v.value).unwrap_or_else(|arc| (*arc).clone()));

            self.db
                .update_db(key, key_hash, value, next_version)
                .unwrap_or_else(|e| panic!("Database error {e}"))
        }
        self.db.inc_next_version();
    }
}

pub fn delete_storage(path: impl AsRef<Path>) {
    fs::remove_dir_all(&path)
        .or_else(|_| fs::remove_file(&path))
        .unwrap();
}

#[cfg(test)]
mod test {
    use jmt::Version;

    use super::*;

    #[derive(Clone)]
    struct TestCase {
        key: StorageKey,
        value: StorageValue,
        version: Version,
    }

    fn create_tests() -> Vec<TestCase> {
        vec![
            TestCase {
                key: StorageKey::from("key_0"),
                value: StorageValue::from("value_0"),
                version: 0,
            },
            TestCase {
                key: StorageKey::from("key_1"),
                value: StorageValue::from("value_1"),
                version: 1,
            },
            TestCase {
                key: StorageKey::from("key_2"),
                value: StorageValue::from("value_2"),
                version: 2,
            },
        ]
    }

    #[test]
    fn test_jmt_storage() {
        let path = schemadb::temppath::TempPath::new();
        let tests = create_tests();
        {
            for test in tests.clone() {
                let mut storage = JmtStorage::with_path(&path).unwrap();
                assert_eq!(storage.db.get_next_version(), test.version);

                storage.set(test.key.clone(), test.value.clone());
                storage.merge();
                storage.finalize();

                assert_eq!(test.value, storage.get(test.key).unwrap());
                assert_eq!(storage.db.get_next_version(), test.version + 1)
            }
        }

        {
            let storage = JmtStorage::with_path(&path).unwrap();
            assert_eq!(storage.db.get_next_version(), tests.len() as u64);
            for test in tests {
                assert_eq!(test.value, storage.get(test.key).unwrap());
            }
        }
    }
}
