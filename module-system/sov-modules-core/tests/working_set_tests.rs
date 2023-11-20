use sov_modules_api::default_context::DefaultContext;
use sov_modules_core::{StateReaderAndWriter, StorageKey, StorageValue, WorkingSet};
use sov_state::codec::BcsCodec;
use sov_state::ProverStorage;

#[test]
fn test_workingset_get() {
    let tempdir = tempfile::tempdir().unwrap();
    let path = tempdir.path();
    let codec = BcsCodec {};
    let storage = ProverStorage::<sov_state::DefaultStorageSpec>::with_path(path).unwrap();

    let prefix = sov_modules_core::Prefix::new(vec![1, 2, 3]);
    let storage_key = StorageKey::new(&prefix, &vec![4, 5, 6], &codec);
    let storage_value = StorageValue::new(&vec![7, 8, 9], &codec);

    let mut working_set = WorkingSet::<DefaultContext>::new(storage.clone());
    working_set.set(&storage_key, storage_value.clone());

    assert_eq!(Some(storage_value), working_set.get(&storage_key));
}
