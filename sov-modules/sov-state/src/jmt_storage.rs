use std::{
    cell::RefCell,
    fs,
    path::Path,
    rc::Rc,
    sync::{Arc, Mutex},
};

use crate::{
    internal_cache::{StorageInternalCache, ValueReader},
    storage::{StorageKey, StorageValue},
    tree_db::TreeReadLogger,
    Storage,
};
use first_read_last_write_cache::cache::FirstReads;
use jmt::{storage::TreeWriter, KeyHash, PhantomHasher, SimpleHasher};
use sovereign_db::state_db::StateDB;

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
pub struct JmtStorage<H: SimpleHasher> {
    cache: Rc<RefCell<StorageInternalCache>>,
    db: StateDB,
    read_logger: Rc<RefCell<Option<TreeReadLogger>>>,
    is_merged: Arc<Mutex<bool>>,
    _phantom_hasher: PhantomHasher<H>,
}

impl<H: SimpleHasher> JmtStorage<H> {
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
            cache: Rc::new(RefCell::new(StorageInternalCache::default())),
            db,
            read_logger: Rc::new(RefCell::new(None)),
            is_merged: Arc::new(Mutex::new(false)),
            _phantom_hasher: Default::default(),
        })
    }

    fn set_merged_true(&self) {
        let mut is_merged = self.is_merged.lock().unwrap();
        *is_merged = true
    }

    /// Gets the first reads from the JmtStorage. Must be preceded by a `merge` call.
    // TODO: combine "get_first_reads" and "take_treedb_log" into a single method, preferably one which
    // can be used as a drop-in replacement for "finalize" when running in prover mode.
    pub fn get_first_reads(&self) -> FirstReads {
        // Sanity check, before getting reads from the batch_cache we have to fill it by calling `merge()`
        let mut is_merged = self.is_merged.lock().unwrap();
        assert!(*is_merged);
        *is_merged = false;

        self.cache.borrow().get_first_reads()
    }

    /// Take the log
    pub fn take_treedb_log(&self) -> Option<TreeReadLogger> {
        self.read_logger.borrow_mut().take()
    }
}

impl<H: SimpleHasher> Storage for JmtStorage<H> {
    fn get(&self, key: StorageKey) -> Option<StorageValue> {
        self.cache.borrow_mut().get_or_fetch(key, &self.db)
    }

    fn set(&mut self, key: StorageKey, value: StorageValue) {
        self.cache.borrow_mut().set(key, value)
    }

    fn delete(&mut self, key: StorageKey) {
        self.cache.borrow_mut().delete(key)
    }

    fn merge(&mut self) {
        self.cache
            .borrow_mut()
            .merge()
            .unwrap_or_else(|e| panic!("Cache merge error: {e}"));
        self.set_merged_true();
    }

    fn merge_reads_and_discard_writes(&mut self) {
        self.cache
            .borrow_mut()
            .merge_reads_and_discard_writes()
            .unwrap_or_else(|e| panic!("Cache merge error: {e}"));
    }

    fn finalize(&mut self) -> [u8; 32] {
        let mut borrowed_cache = self.cache.borrow_mut();
        let slot_cache = borrowed_cache.slot_cache();

        let batch = slot_cache
            .get_all_writes_and_clear_cache()
            .map(|(key, value)| {
                let key_hash = KeyHash(H::hash(key.key.as_ref()));
                self.db
                    .put_preimage(key_hash, key.key.as_ref())
                    .expect("preimage must succeed");
                (
                    key_hash,
                    value.map(|v| Arc::try_unwrap(v.value).unwrap_or_else(|arc| (*arc).clone())),
                )
            });

        let next_version = self.db.get_next_version();
        let mut read_logger_opt = self.read_logger.borrow_mut();
        let read_logger =
            read_logger_opt.get_or_insert_with(|| TreeReadLogger::with_db(self.db.clone()));
        let jmt = jmt::JellyfishMerkleTree::<_, sha2::Sha256>::new(read_logger);

        let (new_root, tree_update) = jmt
            .put_value_set(batch, next_version)
            .expect("JMT update must succeed");

        self.db
            .write_node_batch(&tree_update.node_batch)
            .expect("db write must succeed");
        self.db.inc_next_version();
        new_root.0
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
    use sha2::Sha256;

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
                let mut storage = JmtStorage::<Sha256>::with_path(&path).unwrap();
                assert_eq!(storage.db.get_next_version(), test.version);

                storage.set(test.key.clone(), test.value.clone());
                storage.merge();
                storage.finalize();

                assert_eq!(test.value, storage.get(test.key).unwrap());
                assert_eq!(storage.db.get_next_version(), test.version + 1)
            }
        }

        {
            let storage = JmtStorage::<Sha256>::with_path(&path).unwrap();
            assert_eq!(storage.db.get_next_version(), tests.len() as u64);
            for test in tests {
                assert_eq!(test.value, storage.get(test.key).unwrap());
            }
        }
    }
}
