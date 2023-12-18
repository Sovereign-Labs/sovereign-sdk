mod accessory_map;
mod accessory_value;
mod accessory_vec;

mod kernel_value;
mod versioned_value;

mod map;
mod value;
mod vec;

mod traits;
pub use accessory_map::AccessoryStateMap;
pub use accessory_value::AccessoryStateValue;
pub use accessory_vec::AccessoryStateVec;
pub use kernel_value::KernelStateValue;
pub use map::StateMap;
pub use traits::{
    StateMapAccessor, StateMapError, StateValueAccessor, StateValueError, StateVecAccessor,
    StateVecError,
};
pub use value::StateValue;
pub use vec::StateVec;
pub use versioned_value::VersionedStateValue;

#[cfg(test)]
mod test {
    use jmt::Version;
    use sov_mock_da::{MockBlockHeader, MockDaSpec};
    use sov_modules_core::{StateReaderAndWriter, Storage, StorageKey, StorageValue, WorkingSet};
    use sov_prover_storage_manager::ProverStorageManager;
    use sov_rollup_interface::storage::HierarchicalStorageManager;
    use sov_state::DefaultStorageSpec;

    use crate::default_context::DefaultContext;

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
                version: 1,
            },
            TestCase {
                key: StorageKey::from("key_1"),
                value: StorageValue::from("value_1"),
                version: 2,
            },
            TestCase {
                key: StorageKey::from("key_2"),
                value: StorageValue::from("value_2"),
                version: 3,
            },
            TestCase {
                key: StorageKey::from("key_1"),
                value: StorageValue::from("value_3"),
                version: 4,
            },
        ]
    }

    #[test]
    fn test_jmt_storage() {
        let tempdir = tempfile::tempdir().unwrap();
        let tests = create_tests();
        let storage_config = sov_state::config::Config {
            path: tempdir.path().to_path_buf(),
        };
        {
            let mut storage_manager =
                ProverStorageManager::<MockDaSpec, DefaultStorageSpec>::new(storage_config.clone())
                    .unwrap();
            let header = MockBlockHeader::default();
            let prover_storage = storage_manager.create_storage_on(&header).unwrap();
            for test in tests.clone() {
                {
                    let mut working_set: WorkingSet<DefaultContext> =
                        WorkingSet::new(prover_storage.clone());

                    working_set.set(&test.key, test.value.clone());
                    let (cache, witness) = working_set.checkpoint().freeze();
                    prover_storage
                        .validate_and_commit(cache, &witness)
                        .expect("storage is valid");
                    assert_eq!(
                        test.value,
                        prover_storage.get(&test.key, None, &witness).unwrap()
                    );
                }
            }
            storage_manager
                .save_change_set(&header, prover_storage)
                .unwrap();
            storage_manager.finalize(&header).unwrap();
        }

        {
            let mut storage_manager =
                ProverStorageManager::<MockDaSpec, DefaultStorageSpec>::new(storage_config)
                    .unwrap();
            let header = MockBlockHeader::default();
            let storage = storage_manager.create_storage_on(&header).unwrap();
            for test in tests {
                assert_eq!(
                    test.value,
                    storage
                        .get(&test.key, Some(test.version), &Default::default())
                        .unwrap()
                );
            }
        }
    }

    #[test]
    fn test_restart_lifecycle() {
        let tempdir = tempfile::tempdir().unwrap();
        let storage_config = sov_state::config::Config {
            path: tempdir.path().to_path_buf(),
        };
        {
            let mut storage_manager =
                ProverStorageManager::<MockDaSpec, DefaultStorageSpec>::new(storage_config.clone())
                    .unwrap();
            let header = MockBlockHeader::default();
            let prover_storage = storage_manager.create_storage_on(&header).unwrap();
            assert!(prover_storage.is_empty());
        }

        let key = StorageKey::from("some_key");
        let value = StorageValue::from("some_value");
        // First restart
        {
            let mut storage_manager =
                ProverStorageManager::<MockDaSpec, DefaultStorageSpec>::new(storage_config.clone())
                    .unwrap();
            let header = MockBlockHeader::default();
            let prover_storage = storage_manager.create_storage_on(&header).unwrap();
            assert!(prover_storage.is_empty());
            let mut storage: WorkingSet<DefaultContext> = WorkingSet::new(prover_storage.clone());
            storage.set(&key, value.clone());
            let (cache, witness) = storage.checkpoint().freeze();
            prover_storage
                .validate_and_commit(cache, &witness)
                .expect("storage is valid");
            storage_manager
                .save_change_set(&header, prover_storage)
                .unwrap();
            storage_manager.finalize(&header).unwrap();
        }

        // Correctly restart from disk
        {
            let mut storage_manager =
                ProverStorageManager::<MockDaSpec, DefaultStorageSpec>::new(storage_config.clone())
                    .unwrap();
            let prover_storage = storage_manager.create_finalized_storage().unwrap();
            assert!(!prover_storage.is_empty());
            assert_eq!(
                value,
                prover_storage.get(&key, None, &Default::default()).unwrap()
            );
        }
    }
}
