use first_read_last_write_cache::cache::{self, FirstReads};
use sov_state::storage::{StorageKey, StorageValue};
/// `ValueReader` Reads a value from an external data source.
pub trait ValueReader {
    fn read_value(&self, key: StorageKey) -> Option<StorageValue>;
}

pub type JmtDb = sovereign_db::state_db::StateDB;

impl ValueReader for JmtDb {
    fn read_value(&self, key: StorageKey) -> Option<StorageValue> {
        match self.get_value_option_by_key(0, key.as_ref()) {
            Ok(value) => value.map(StorageValue::new_from_bytes),
            Err(e) => panic!("Unable to read value from db: {e}"),
        }
    }
}

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
