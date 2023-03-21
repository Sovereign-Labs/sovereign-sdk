use std::path::Path;

use super::*;
use crate::{mocks::MockStorageSpec, ProverStorage};

enum Operation {
    Merge,
    Finalize,
}

impl Operation {
    fn execute(&self, working_set: &mut WorkingSet<ProverStorage<MockStorageSpec>>) {
        match self {
            Operation::Merge => working_set.commit(),
            Operation::Finalize => {
                let (cache_log, witness) = working_set.freeze();
                let db = working_set.backing();
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
    fn execute(&self, working_set: &mut WorkingSet<ProverStorage<MockStorageSpec>>) {
        for op in self.operations.iter() {
            op.execute(working_set)
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
    let mut working_set = WorkingSet::new(ProverStorage::with_path(&path).unwrap());

    let mut state_map = StateMap::new(Prefix::new(vec![0]));
    state_map.set(&key, value, &mut working_set);
    (state_map, working_set)
}

#[test]
fn test_state_map() {
    let path = schemadb::temppath::TempPath::new();
    for (before_remove, after_remove) in create_storage_operations() {
        let key = 1;
        let value = 11;
        let (mut state_map, mut working_set) = create_state_map_and_storage(key, value, &path);

        before_remove.execute(&mut working_set);
        assert_eq!(state_map.remove(&key, &mut working_set).unwrap(), value);

        after_remove.execute(&mut working_set);
        assert!(state_map.get(&key, &mut working_set).is_none())
    }
}

fn create_state_value_and_storage(
    value: u32,
    path: impl AsRef<Path>,
) -> (
    StateValue<u32, ProverStorage<MockStorageSpec>>,
    WorkingSet<ProverStorage<MockStorageSpec>>,
) {
    let mut working_set = WorkingSet::new(ProverStorage::with_path(&path).unwrap());

    let mut state_value = StateValue::new(Prefix::new(vec![0]));
    state_value.set(value, &mut working_set);
    (state_value, working_set)
}

#[test]
fn test_state_value() {
    let path = schemadb::temppath::TempPath::new();
    for (before_remove, after_remove) in create_storage_operations() {
        let value = 11;
        let (mut state_value, mut working_set) = create_state_value_and_storage(value, &path);

        before_remove.execute(&mut working_set);
        assert_eq!(state_value.remove(&mut working_set).unwrap(), value);

        after_remove.execute(&mut working_set);
        assert!(state_value.get(&mut working_set).is_none())
    }
}
