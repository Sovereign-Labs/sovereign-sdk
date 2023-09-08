use std::path::Path;

use borsh::{BorshDeserialize, BorshSerialize};
use sov_state::*;

enum Operation {
    Merge,
    Finalize,
}

const EMPTY_ROOT: [u8; 32] = *b"SPARSE_MERKLE_PLACEHOLDER_HASH__";

impl Operation {
    fn execute<S: Storage>(&self, working_set: WorkingSet<S>) -> StateCheckpoint<S> {
        match self {
            Operation::Merge => working_set.checkpoint(),
            Operation::Finalize => {
                let db = working_set.backing().clone();
                let (cache_log, witness) = working_set.checkpoint().freeze();

                db.validate_and_commit(cache_log, &witness)
                    .expect("JMT update is valid");

                StateCheckpoint::new(db)
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
            working_set = op.execute(working_set).to_revertable()
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
    state_map.set(&key, &value, &mut working_set);
    (state_map, working_set)
}

#[test]
fn test_state_map_with_remove() {
    let tempdir = tempfile::tempdir().unwrap();
    let path = tempdir.path();
    for (before_remove, after_remove) in create_storage_operations() {
        let key = 1;
        let value = 11;
        let (state_map, mut working_set) = create_state_map_and_storage(key, value, path);

        working_set = before_remove.execute(working_set);
        assert_eq!(state_map.remove(&key, &mut working_set).unwrap(), value);

        working_set = after_remove.execute(working_set);
        assert!(state_map.get(&key, &mut working_set).is_none());
    }
}

#[test]
fn test_state_map_with_delete() {
    let tempdir = tempfile::tempdir().unwrap();
    let path = tempdir.path();
    for (before_delete, after_delete) in create_storage_operations() {
        let key = 1;
        let value = 11;
        let (state_map, mut working_set) = create_state_map_and_storage(key, value, path);

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
    state_value.set(&value, &mut working_set);
    (state_value, working_set)
}

#[test]
fn test_state_value_with_remove() {
    let tempdir = tempfile::tempdir().unwrap();
    let path = tempdir.path();
    for (before_remove, after_remove) in create_storage_operations() {
        let value = 11;
        let (state_value, mut working_set) = create_state_value_and_storage(value, path);

        working_set = before_remove.execute(working_set);
        assert_eq!(state_value.remove(&mut working_set).unwrap(), value);

        working_set = after_remove.execute(working_set);
        assert!(state_value.get(&mut working_set).is_none());
    }
}

#[test]
fn test_state_value_with_delete() {
    let tempdir = tempfile::tempdir().unwrap();
    let path = tempdir.path();
    for (before_delete, after_delete) in create_storage_operations() {
        let value = 11;
        let (state_value, mut working_set) = create_state_value_and_storage(value, path);

        working_set = before_delete.execute(working_set);
        state_value.delete(&mut working_set);

        working_set = after_delete.execute(working_set);
        assert!(state_value.get(&mut working_set).is_none());
    }
}

#[test]
fn test_witness_roundtrip() {
    let tempdir = tempfile::tempdir().unwrap();
    let path = tempdir.path();
    let state_value = StateValue::new(Prefix::new(vec![0]));

    // Native execution
    let witness: ArrayWitness = {
        let storage = ProverStorage::<DefaultStorageSpec>::with_path(path).unwrap();
        let mut working_set = WorkingSet::new(storage.clone());
        state_value.set(&11, &mut working_set);
        let _ = state_value.get(&mut working_set);
        state_value.set(&22, &mut working_set);
        let (cache_log, witness) = working_set.checkpoint().freeze();

        let _ = storage
            .validate_and_commit(cache_log, &witness)
            .expect("Native jmt validation should succeed");
        witness
    };

    {
        let storage = ZkStorage::<DefaultStorageSpec>::new(EMPTY_ROOT);
        let mut working_set = WorkingSet::with_witness(storage.clone(), witness);
        state_value.set(&11, &mut working_set);
        let _ = state_value.get(&mut working_set);
        state_value.set(&22, &mut working_set);
        let (cache_log, witness) = working_set.checkpoint().freeze();

        let _ = storage
            .validate_and_commit(cache_log, &witness)
            .expect("ZK validation should succeed");
    };
}

fn create_state_vec_and_storage<T: BorshDeserialize + BorshSerialize>(
    values: Vec<T>,
    path: impl AsRef<Path>,
) -> (StateVec<T>, WorkingSet<ProverStorage<DefaultStorageSpec>>) {
    let mut working_set = WorkingSet::new(ProverStorage::with_path(&path).unwrap());

    let state_vec = StateVec::new(Prefix::new(vec![0]));
    state_vec.set_all(values, &mut working_set);
    (state_vec, working_set)
}

#[test]
fn test_state_vec_len() {
    let tempdir = tempfile::tempdir().unwrap();
    let path = tempdir.path();
    for (before_len, after_len) in create_storage_operations() {
        let values = vec![11, 22, 33];
        let (state_vec, mut working_set) = create_state_vec_and_storage(values.clone(), path);

        working_set = before_len.execute(working_set);

        working_set = after_len.execute(working_set);

        assert_eq!(state_vec.len(&mut working_set), values.len());
    }
}

#[test]
fn test_state_vec_get() {
    let tempdir = tempfile::tempdir().unwrap();
    let path = tempdir.path();
    for (before_get, after_get) in create_storage_operations() {
        let values = vec![56, 55, 54];
        let (state_vec, mut working_set) = create_state_vec_and_storage(values.clone(), path);

        working_set = before_get.execute(working_set);

        let val = state_vec.get(1, &mut working_set);
        let err_val = state_vec.get_or_err(3, &mut working_set);
        assert!(val.is_some());
        assert!(err_val.is_err());

        let val = val.unwrap();
        assert_eq!(val, values.get(1).unwrap().clone());

        working_set = after_get.execute(working_set);
        let val = state_vec.get(1, &mut working_set);
        let err_val = state_vec.get_or_err(3, &mut working_set);
        assert!(val.is_some());
        assert!(err_val.is_err());

        let val = val.unwrap();
        assert_eq!(val, values.get(1).unwrap().clone());
    }
}

#[test]
fn test_state_vec_set() {
    let tempdir = tempfile::tempdir().unwrap();
    let path = tempdir.path();
    for (before_set, after_set) in create_storage_operations() {
        let values = vec![56, 55, 54];
        let (state_vec, mut working_set) = create_state_vec_and_storage(values.clone(), path);

        working_set = before_set.execute(working_set);
        let val = state_vec.set(1, &99, &mut working_set);
        assert!(val.is_ok());

        let val_err = state_vec.set(3, &99, &mut working_set);
        assert!(val_err.is_err());

        working_set = after_set.execute(working_set);

        let val = state_vec.get(1, &mut working_set);
        let err_val = state_vec.get_or_err(3, &mut working_set);

        assert!(val.is_some());
        assert!(err_val.is_err());

        let val = val.unwrap();
        assert_eq!(val, 99);
    }
}

#[test]
fn test_state_vec_push() {
    let tempdir = tempfile::tempdir().unwrap();
    let path = tempdir.path();
    for (before_push, after_push) in create_storage_operations() {
        let values = vec![56, 55, 54];
        let (state_vec, mut working_set) = create_state_vec_and_storage(values.clone(), path);

        working_set = before_push.execute(working_set);

        state_vec.push(&53, &mut working_set);

        working_set = after_push.execute(working_set);

        let len = state_vec.len(&mut working_set);
        assert_eq!(len, 4);

        let val = state_vec.get(3, &mut working_set);
        assert!(val.is_some());

        let val = val.unwrap();
        assert_eq!(val, 53);
    }
}

#[test]
fn test_state_vec_pop() {
    let tempdir = tempfile::tempdir().unwrap();
    let path = tempdir.path();
    for (before_pop, after_pop) in create_storage_operations() {
        let values = vec![56, 55, 54];
        let (state_vec, mut working_set) = create_state_vec_and_storage(values.clone(), path);

        working_set = before_pop.execute(working_set);

        let popped = state_vec.pop(&mut working_set);

        assert_eq!(popped.unwrap(), 54);

        working_set = after_pop.execute(working_set);

        let len = state_vec.len(&mut working_set);
        assert_eq!(len, 2);

        let val = state_vec.get(1, &mut working_set);
        assert!(val.is_some());

        let val = val.unwrap();
        assert_eq!(val, 55);
    }
}

#[test]
fn test_state_vec_set_all() {
    let tempdir = tempfile::tempdir().unwrap();
    let path = tempdir.path();
    for (before_set_all, after_set_all) in create_storage_operations() {
        let values = vec![56, 55, 54];
        let (state_vec, mut working_set) = create_state_vec_and_storage(values.clone(), path);

        working_set = before_set_all.execute(working_set);

        let new_values: Vec<u32> = vec![1];
        state_vec.set_all(new_values, &mut working_set);

        working_set = after_set_all.execute(working_set);

        let val = state_vec.get(0, &mut working_set);

        assert!(val.is_some());

        let val = val.unwrap();
        assert_eq!(val, 1);

        let len = state_vec.len(&mut working_set);
        assert_eq!(len, 1);

        let val = state_vec.get_or_err(1, &mut working_set);

        assert!(val.is_err());
    }
}

#[test]
fn test_state_vec_diff_type() {
    let tempdir = tempfile::tempdir().unwrap();
    let path = tempdir.path();
    for (before_ops, after_ops) in create_storage_operations() {
        let values = vec![String::from("Hello"), String::from("World")];
        let (state_vec, mut working_set) = create_state_vec_and_storage(values.clone(), path);

        working_set = before_ops.execute(working_set);

        let val0 = state_vec.get(0, &mut working_set);
        let val1 = state_vec.pop(&mut working_set);
        state_vec.push(&String::from("new str"), &mut working_set);

        working_set = after_ops.execute(working_set);

        assert!(val0.is_some());
        assert!(val1.is_some());

        let val0 = val0.unwrap();
        let val1 = val1.unwrap();
        assert_eq!(val0, String::from("Hello"));
        assert_eq!(val1, String::from("World"));

        let val = state_vec.get(1, &mut working_set);
        assert!(val.is_some());

        let val = val.unwrap();
        assert_eq!(val, String::from("new str"));

        let len = state_vec.len(&mut working_set);
        assert_eq!(len, 2);
    }
}
