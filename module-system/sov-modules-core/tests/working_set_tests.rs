use sov_mock_da::MockDaSpec;
use sov_modules_api::default_context::DefaultContext;
use sov_modules_core::capabilities::mocks::MockKernel;
use sov_modules_core::{
    Address, Context, KernelWorkingSet, StateReaderAndWriter, StorageKey, StorageValue, WorkingSet,
};
use sov_prover_storage_manager::new_orphan_storage;
use sov_state::codec::BcsCodec;

#[test]
fn test_workingset_get() {
    let tempdir = tempfile::tempdir().unwrap();
    let codec = BcsCodec {};
    let storage = new_orphan_storage(tempdir.path()).unwrap();

    let prefix = sov_modules_core::Prefix::new(vec![1, 2, 3]);
    let storage_key = StorageKey::new(&prefix, &vec![4, 5, 6], &codec);
    let storage_value = StorageValue::new(&vec![7, 8, 9], &codec);

    let mut working_set = WorkingSet::<DefaultContext>::new(storage.clone());
    working_set.set(&storage_key, storage_value.clone());

    assert_eq!(Some(storage_value), working_set.get(&storage_key));
}

#[test]
fn test_versioned_workingset_get() {
    let tempdir = tempfile::tempdir().unwrap();
    let codec = BcsCodec {};
    let storage = new_orphan_storage(tempdir.path()).unwrap();

    let prefix = sov_modules_core::Prefix::new(vec![1, 2, 3]);
    let storage_key = StorageKey::new(&prefix, &vec![4, 5, 6], &codec);
    let storage_value = StorageValue::new(&vec![7, 8, 9], &codec);

    let sender = Address::from([1; 32]);
    let sequencer = Address::from([2; 32]);
    let mut working_set = WorkingSet::<DefaultContext>::new(storage.clone());
    let mut working_set = working_set.versioned_state(&DefaultContext::new(sender, sequencer, 1));
    working_set.set(&storage_key, storage_value.clone());

    assert_eq!(Some(storage_value), working_set.get(&storage_key));
}

#[test]
fn test_kernel_workingset_get() {
    let tempdir = tempfile::tempdir().unwrap();
    let codec = BcsCodec {};
    let storage = new_orphan_storage(tempdir.path()).unwrap();

    let prefix = sov_modules_core::Prefix::new(vec![1, 2, 3]);
    let storage_key = StorageKey::new(&prefix, &vec![4, 5, 6], &codec);
    let storage_value = StorageValue::new(&vec![7, 8, 9], &codec);
    let kernel: MockKernel<DefaultContext, MockDaSpec> = MockKernel::new(4, 1);

    let mut working_set = WorkingSet::<DefaultContext>::new(storage.clone());
    let mut working_set = KernelWorkingSet::from_kernel(&kernel, &mut working_set);
    working_set.set(&storage_key, storage_value.clone());

    assert_eq!(Some(storage_value), working_set.get(&storage_key));
}
