use crate::{
    mocks::MockStorageSpec,
    storage::{StorageKey, StorageValue},
    ProverStorage, Storage, WorkingSet, ZkStorage,
};

#[test]
fn test_value_absent_in_zk_storage() {
    let key = StorageKey::from("key");
    let value = StorageValue::from("value");

    let path = schemadb::temppath::TempPath::new();
    let witness = {
        let backing_store = ProverStorage::<MockStorageSpec>::with_path(&path).unwrap();
        let mut tx_store = WorkingSet::new(backing_store);

        tx_store.set(key.clone(), value.clone());
        let (_, witness) = tx_store.freeze();
        witness
    };

    {
        // Here we crate a new ZkStorage with an empty inner cache.
        let storage = ZkStorage::<MockStorageSpec>::new([0u8; 32]);
        // `storage.get` tries to fetch the value from the (empty) inner cache but it fails,
        // then it fallbacks to the `reads` we provided in the constructor of the ZkStorage.
        let retrieved_value = storage.get(key, &witness);
        assert_eq!(Some(value), retrieved_value);
    }
}
