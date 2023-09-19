mod containers;
mod scratchpad;

pub use containers::*;
pub use scratchpad::*;

#[cfg(test)]
mod test {
    use jmt::Version;
    use sov_state::storage::{Storage, StorageKey, StorageValue};
    use sov_state::{DefaultStorageSpec, ProverStorage};

    use crate::default_context::DefaultContext;
    use crate::{StateReaderAndWriter, WorkingSet};

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
        ]
    }

    #[test]
    fn test_jmt_storage() {
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path();
        let tests = create_tests();
        {
            for test in tests.clone() {
                let prover_storage = ProverStorage::<DefaultStorageSpec>::with_path(path).unwrap();
                let mut storage: WorkingSet<DefaultContext> =
                    WorkingSet::new(prover_storage.clone());
                assert_eq!(prover_storage.db().get_next_version(), test.version);

                storage.set(&test.key, test.value.clone());
                let (cache, witness) = storage.checkpoint().freeze();
                prover_storage
                    .validate_and_commit(cache, &witness)
                    .expect("storage is valid");

                assert_eq!(test.value, prover_storage.get(&test.key, &witness).unwrap());
                assert_eq!(prover_storage.db().get_next_version(), test.version + 1)
            }
        }

        {
            let storage = ProverStorage::<DefaultStorageSpec>::with_path(path).unwrap();
            assert_eq!(storage.db().get_next_version(), (tests.len() + 1) as u64);
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
