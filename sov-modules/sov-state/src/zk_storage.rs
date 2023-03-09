use std::{cell::RefCell, rc::Rc, sync::Arc};

use first_read_last_write_cache::cache::{self, FirstReads};
use jmt::{KeyHash, PhantomHasher, SimpleHasher};

use crate::{
    internal_cache::{StorageInternalCache, ValueReader},
    storage::{StorageKey, StorageValue},
    tree_db::ZkTreeDb,
    Storage,
};

// Implementation of `ValueReader` trait for the zk-context. FirstReads is backed by a HashMap internally,
// this is a good default choice. Once we start integrating with a proving system
// we might want to explore other alternatives. For example, in Risc0 we could implement `ValueReader`
// in terms of `env::read()` and fetch values lazily from the host.
impl ValueReader for FirstReads {
    fn read_value(&self, key: StorageKey) -> Option<StorageValue> {
        let key = key.as_cache_key();
        match self.get(&key) {
            cache::ValueExists::Yes(read) => read.map(StorageValue::new_from_cache_value),
            // It is ok to panic here, `ZkStorage` must be able to access all the keys it needs.
            cache::ValueExists::No => panic!("Error: Key {key:?} is inaccessible"),
        }
    }
}

#[derive(Clone)]
pub struct ZkStorage<H: SimpleHasher> {
    cache: Rc<RefCell<StorageInternalCache>>,
    tree_reader: Rc<ZkTreeDb>,
    value_reader: FirstReads,
    _phantom_hasher: PhantomHasher<H>,
}

impl<H: SimpleHasher> ZkStorage<H> {
    pub fn new(value_reader: FirstReads, tree_reader: ZkTreeDb) -> Self {
        Self {
            value_reader,
            cache: Rc::new(RefCell::new(StorageInternalCache::default())),
            tree_reader: Rc::new(tree_reader),
            _phantom_hasher: Default::default(),
        }
    }

    #[cfg(test)]
    pub fn non_finalizable(value_reader: FirstReads) -> Self {
        Self {
            value_reader,
            cache: Rc::new(RefCell::new(StorageInternalCache::default())),
            tree_reader: Rc::new(ZkTreeDb::empty()),
            _phantom_hasher: Default::default(),
        }
    }
}

impl<H: SimpleHasher> Storage for ZkStorage<H> {
    fn get(&self, key: StorageKey) -> Option<StorageValue> {
        self.cache
            .borrow_mut()
            .get_or_fetch(key, &self.value_reader)
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
    }

    fn merge_reads_and_discard_writes(&mut self) {
        self.cache
            .borrow_mut()
            .merge_reads_and_discard_writes()
            .unwrap_or_else(|e| panic!("Cache merge error: {e}"));
    }

    fn finalize(&mut self) -> [u8; 32] {
        let jmt = jmt::JellyfishMerkleTree::<_, H>::new(self.tree_reader.as_ref());
        let mut cache = self.cache.borrow_mut();
        cache.merge().expect("cache must be valid");
        let value_set = cache
            .slot_cache()
            .get_all_writes_and_clear_cache()
            .map(|(key, value)| {
                // TODO: Allow jmt to work on borrowed and/or ref counted data
                let key_hash = KeyHash(H::hash(key.key.as_ref()));

                (
                    key_hash,
                    value.map(|v| Arc::try_unwrap(v.value).unwrap_or_else(|arc| (*arc).clone())),
                )
            });
        let (root, _) = jmt
            .put_value_set(value_set, self.tree_reader.next_version)
            .expect("jmt update should succeed");

        root.0
    }
}
