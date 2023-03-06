use std::{fs, path::Path, sync::Arc};

use crate::{
    internal_cache::{StorageInternalCache, ValueReader},
    storage::{StorageKey, StorageValue},
    Storage,
};
use first_read_last_write_cache::cache::FirstReads;
use jmt::{
    storage::{NodeBatch, TreeWriter},
    KeyHash,
};
use sovereign_db::state_db::StateDB;
use sovereign_sdk::core::crypto;

impl ValueReader for StateDB {
    fn read_value(&self, key: StorageKey) -> Option<StorageValue> {
        match self.get_value_option_by_key(0, key.as_ref()) {
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
        Self {
            batch_cache: StorageInternalCache::default(),
            tx_cache: StorageInternalCache::default(),
            db: StateDB::temporary(),
        }
    }

    pub fn with_path(path: impl AsRef<Path>) -> Result<Self, anyhow::Error> {
        let db = StateDB::with_path(&path)?;
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

    fn finalize(&mut self) {
        let mut batch = NodeBatch::default();
        let cache = &mut self.batch_cache.borrow_mut();

        let mut data = Vec::with_capacity(cache.len());

        for (cache_key, cache_value) in cache.drain() {
            let key = &cache_key.key;
            // TODO: Don't hardcode the hashing algorithm
            // https://github.com/Sovereign-Labs/sovereign/issues/113
            let key_hash = KeyHash(crypto::hash::sha2(key.as_ref()).0);

            self.db
                .put_preimage(key_hash, key)
                .unwrap_or_else(|e| panic!("Database error: {e}"));

            let value = cache_value.map(|v| Arc::try_unwrap(v.value).unwrap());
            // TODO: Bump and save `version` number
            // https://github.com/Sovereign-Labs/sovereign/issues/114
            data.push(((0, key_hash), value));
        }

        batch.extend(vec![], data);
        self.db.write_node_batch(&batch).unwrap();
    }
}

pub fn delete_storage(path: impl AsRef<Path>) {
    fs::remove_dir_all(&path)
        .or_else(|_| fs::remove_file(&path))
        .unwrap_or(());
}
