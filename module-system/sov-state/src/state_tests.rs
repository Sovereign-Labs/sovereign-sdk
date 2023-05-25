use std::path::Path;

use super::*;
use crate::{ArrayWitness, DefaultStorageSpec, ProverStorage};

enum Operation {
    Merge,
    Finalize,
}

const EMPTY_ROOT: [u8; 32] = *b"SPARSE_MERKLE_PLACEHOLDER_HASH__";

impl Operation {
    fn execute<S: Storage>(&self, mut working_set: WorkingSet<S>) -> WorkingSet<S> {
        match self {
            Operation::Merge => working_set.commit(),
            Operation::Finalize => {
                let (cache_log, witness) = working_set.freeze();
                let db = working_set.backing();
                db.validate_and_commit(cache_log, &witness)
                    .expect("JMT update is valid");

                working_set
            }
        }
    }
}

struct StorageOperation {
    operations: Vec<Operation>,
}

impl StorageOperation {
    fn execute<S: Storage>(&self, mut working_set: WorkingSet<S>) -> WorkingSet<S> {
        for op in self.operations.iter() {
            working_set = op.execute(working_set)
        }
        working_set
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
    StateMap<u32, u32>,
    WorkingSet<ProverStorage<DefaultStorageSpec>>,
) {
    let mut working_set = WorkingSet::new(ProverStorage::with_path(&path).unwrap());

    let state_map = StateMap::new(Prefix::new(vec![0]));
    state_map.set(&key, value, &mut working_set);
    (state_map, working_set)
}

#[test]
fn test_state_map_with_remove() {
    let path = sov_schema_db::temppath::TempPath::new();
    for (before_remove, after_remove) in create_storage_operations() {
        let key = 1;
        let value = 11;
        let (state_map, mut working_set) = create_state_map_and_storage(key, value, &path);

        working_set = before_remove.execute(working_set);
        assert_eq!(state_map.remove(&key, &mut working_set).unwrap(), value);

        working_set = after_remove.execute(working_set);
        assert!(state_map.get(&key, &mut working_set).is_none());
    }
}

#[test]
fn test_state_map_with_delete() {
    let path = sov_schema_db::temppath::TempPath::new();
    for (before_delete, after_delete) in create_storage_operations() {
        let key = 1;
        let value = 11;
        let (state_map, mut working_set) = create_state_map_and_storage(key, value, &path);

        working_set = before_delete.execute(working_set);
        state_map.delete(&key, &mut working_set);

        working_set = after_delete.execute(working_set);
        assert!(state_map.get(&key, &mut working_set).is_none());
    }
}

fn create_state_value_and_storage(
    value: u32,
    path: impl AsRef<Path>,
) -> (
    StateValue<u32>,
    WorkingSet<ProverStorage<DefaultStorageSpec>>,
) {
    let mut working_set = WorkingSet::new(ProverStorage::with_path(&path).unwrap());

    let state_value = StateValue::new(Prefix::new(vec![0]));
    state_value.set(value, &mut working_set);
    (state_value, working_set)
}

#[test]
fn test_state_value_with_remove() {
    let path = sov_schema_db::temppath::TempPath::new();
    for (before_remove, after_remove) in create_storage_operations() {
        let value = 11;
        let (state_value, mut working_set) = create_state_value_and_storage(value, &path);

        working_set = before_remove.execute(working_set);
        assert_eq!(state_value.remove(&mut working_set).unwrap(), value);

        working_set = after_remove.execute(working_set);
        assert!(state_value.get(&mut working_set).is_none());
    }
}

#[test]
fn test_state_value_with_delete() {
    let path = sov_schema_db::temppath::TempPath::new();
    for (before_delete, after_delete) in create_storage_operations() {
        let value = 11;
        let (state_value, mut working_set) = create_state_value_and_storage(value, &path);

        working_set = before_delete.execute(working_set);
        state_value.delete(&mut working_set);

        working_set = after_delete.execute(working_set);
        assert!(state_value.get(&mut working_set).is_none());
    }
}

#[test]
fn test_witness_roundtrip() {
    let path: sov_schema_db::temppath::TempPath = sov_schema_db::temppath::TempPath::new();
    let state_value = StateValue::new(Prefix::new(vec![0]));

    // Native execution
    let witness: ArrayWitness = {
        let storage = ProverStorage::<DefaultStorageSpec>::with_path(&path).unwrap();
        let mut working_set = WorkingSet::new(storage.clone());
        state_value.set(11, &mut working_set);
        let _ = state_value.get(&mut working_set);
        state_value.set(22, &mut working_set);
        let (cache_log, witness) = working_set.freeze();

        let _ = storage
            .validate_and_commit(cache_log, &witness)
            .expect("Native jmt validation should succeed");
        witness
    };

    {
        let storage = ZkStorage::<DefaultStorageSpec>::new(EMPTY_ROOT);
        let mut working_set = WorkingSet::with_witness(storage.clone(), witness);
        state_value.set(11, &mut working_set);
        let _ = state_value.get(&mut working_set);
        state_value.set(22, &mut working_set);
        let (cache_log, witness) = working_set.freeze();

        let _ = storage
            .validate_and_commit(cache_log, &witness)
            .expect("ZK validation should succeed");
    };
}
