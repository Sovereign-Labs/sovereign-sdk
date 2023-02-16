use first_read_last_write_cache::cache::FirstReads;
use std::collections::HashMap;

use crate::{
    storage::{StorageKey, StorageValue},
    Storage, ZkStorage,
};

#[test]
fn test_value_absent_in_zk_storage() {
    let key = StorageKey::from("key");
    let value = Some(StorageValue::from("value"));

    // TODO: For now we crate the FirstReads manually. Once we have
    // JmtDB ready, we should update the test to use JmtStorage instead.
    let reads = make_reads(key.clone(), value.clone());

    // Here we crate a new ZkStorage with an empty inner cache.
    let storage = ZkStorage::new(reads);
    // `storage.get` tries to fetch the value from the (empty) inner cache but it fails,
    // then it fallbacks to the `reads` we provided in the constructor of the ZkStorage.
    let retrieved_value = storage.get(key);
    assert_eq!(value, retrieved_value);
}

fn make_reads(key: StorageKey, value: Option<StorageValue>) -> FirstReads {
    let mut reads = HashMap::default();
    reads.insert(key.as_cache_key(), value.map(|v| v.as_cache_value()));
    FirstReads::new(reads)
}
