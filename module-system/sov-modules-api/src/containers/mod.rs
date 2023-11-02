mod accessory_map;
mod accessory_value;
mod accessory_vec;

mod map;
mod value;
mod vec;

pub use accessory_map::AccessoryStateMap;
pub use accessory_value::AccessoryStateValue;
pub use accessory_vec::AccessoryStateVec;
pub use map::{StateMap, StateMapError};
pub use value::StateValue;
pub use vec::{Error as StateVecError, StateVec};

#[cfg(test)]
mod test {
    use jmt::Version;
    use sov_modules_core::{StateReaderAndWriter, Storage, StorageKey, StorageValue, WorkingSet};
    use sov_state::{DefaultStorageSpec, ProverStorage};

    use crate::default_context::DefaultContext;

    #[derive(Clone)]
    struct TestCase {
        key: StorageKey,
        value: StorageValue,
        version: Version,
    }

    fn get_state_db_version(path: &std::path::Path) -> Version {
        let state_db = sov_db::state_db::StateDB::with_path(path).unwrap();
        state_db.get_next_version()
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
        ]
    }

    #[test]
    fn test_jmt_storage() {
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path();
        let tests = create_tests();
        {
            for test in tests.clone() {
                let version_before = get_state_db_version(path);
                assert_eq!(version_before, test.version);
                {
                    let prover_storage =
                        ProverStorage::<DefaultStorageSpec>::with_path(path).unwrap();
                    let mut working_set: WorkingSet<DefaultContext> =
                        WorkingSet::new(prover_storage.clone());

                    working_set.set(&test.key, test.value.clone());
                    let (cache, witness) = working_set.checkpoint().freeze();
                    prover_storage
                        .validate_and_commit(cache, &witness)
                        .expect("storage is valid");
                    assert_eq!(test.value, prover_storage.get(&test.key, &witness).unwrap());
                }
                let version_after = get_state_db_version(path);
                assert_eq!(version_after, test.version + 1)
            }
        }

        {
            let version_from_db = get_state_db_version(path);
            let storage = ProverStorage::<DefaultStorageSpec>::with_path(path).unwrap();
            assert_eq!(version_from_db, (tests.len() + 1) as u64);
            for test in tests {
                assert_eq!(
                    test.value,
                    storage.get(&test.key, &Default::default()).unwrap()
                );
            }
        }
    }

    #[test]
    fn test_restart_lifecycle() {
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path();
        {
            let prover_storage = ProverStorage::<DefaultStorageSpec>::with_path(path).unwrap();
            assert!(prover_storage.is_empty());
        }

        let key = StorageKey::from("some_key");
        let value = StorageValue::from("some_value");
        // First restart
        {
            let prover_storage = ProverStorage::<DefaultStorageSpec>::with_path(path).unwrap();
            assert!(prover_storage.is_empty());
            let mut storage: WorkingSet<DefaultContext> = WorkingSet::new(prover_storage.clone());
            storage.set(&key, value.clone());
            let (cache, witness) = storage.checkpoint().freeze();
            prover_storage
                .validate_and_commit(cache, &witness)
                .expect("storage is valid");
        }

        // Correctly restart from disk
        {
            let prover_storage = ProverStorage::<DefaultStorageSpec>::with_path(path).unwrap();
            assert!(!prover_storage.is_empty());
            assert_eq!(
                value,
                prover_storage.get(&key, &Default::default()).unwrap()
            );
        }
    }
}
