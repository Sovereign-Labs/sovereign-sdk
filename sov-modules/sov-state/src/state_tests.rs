use std::path::Path;

use super::*;
use crate::{mocks::MockStorageSpec, ProverStorage};

enum Operation {
    Merge,
    Finalize,
}

impl Operation {
    fn execute(&self, storage: &mut WorkingSet<ProverStorage<MockStorageSpec>>) {
        match self {
            Operation::Merge => storage.commit(),
            Operation::Finalize => {
                let db = storage.backing();
                let (cache_log, witness) = storage.freeze();
                db.validate_and_commit(cache_log, &witness)
                    .expect("JMT update is valid");
            }
        }
    }
}

struct StorageOperation {
    operations: Vec<Operation>,
}

impl StorageOperation {
    fn execute(&self, storage: WorkingSet<ProverStorage<MockStorageSpec>>) {
        for op in self.operations.iter() {
            op.execute(&mut storage.clone())
        }
    }
}

fn create_storage_operations() -> Vec<(StorageOperation, StorageOperation)> {
    // Test cases for various interweavings of storage operations.
    vec![
        (
            StorageOperation { operations: vec![] },
            StorageOperation { operations: vec![] },
        ),
        (
            StorageOperation {
                operations: vec![Operation::Merge],
            },
            StorageOperation { operations: vec![] },
        ),
        (
            StorageOperation {
                operations: vec![Operation::Merge, Operation::Finalize],
            },
            StorageOperation { operations: vec![] },
        ),
        (
            StorageOperation {
                operations: vec![Operation::Merge],
            },
            StorageOperation {
                operations: vec![Operation::Finalize],
            },
        ),
        (
            StorageOperation { operations: vec![] },
            StorageOperation {
                operations: vec![Operation::Merge, Operation::Finalize],
            },
        ),
    ]
}

fn create_state_map_and_storage(
    key: u32,
    value: u32,
    path: impl AsRef<Path>,
) -> (
    StateMap<u32, u32, ProverStorage<MockStorageSpec>>,
    WorkingSet<ProverStorage<MockStorageSpec>>,
) {
    let storage = WorkingSet::new(ProverStorage::with_path(&path).unwrap());

    let mut state_map = StateMap::new(storage.clone(), Prefix::new(vec![0]));
    state_map.set(&key, value);
    (state_map, storage)
}

#[test]
fn test_state_map() {
    let path = schemadb::temppath::TempPath::new();
    for (before_remove, after_remove) in create_storage_operations() {
        let key = 1;
        let value = 11;
        let (mut state_map, storage) = create_state_map_and_storage(key, value, &path);

        before_remove.execute(storage.clone());
        assert_eq!(state_map.remove(&key).unwrap(), value);

        after_remove.execute(storage);
        assert!(state_map.get(&key).is_none())
    }
}

fn create_state_value_and_storage(
    value: u32,
    path: impl AsRef<Path>,
) -> (
    StateValue<u32, ProverStorage<MockStorageSpec>>,
    WorkingSet<ProverStorage<MockStorageSpec>>,
) {
    let storage = WorkingSet::new(ProverStorage::with_path(&path).unwrap());

    let mut state_value = StateValue::new(storage.clone(), Prefix::new(vec![0]));
    state_value.set(value);
    (state_value, storage)
}

#[test]
fn test_state_value() {
    let path = schemadb::temppath::TempPath::new();
    for (before_remove, after_remove) in create_storage_operations() {
        let value = 11;
        let (mut state_value, storage) = create_state_value_and_storage(value, &path);

        before_remove.execute(storage.clone());
        assert_eq!(state_value.remove().unwrap(), value);

        after_remove.execute(storage);
        assert!(state_value.get().is_none())
    }
}
