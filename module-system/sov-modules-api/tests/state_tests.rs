use borsh::{BorshDeserialize, BorshSerialize};
use sov_modules_api::default_context::{DefaultContext, ZkDefaultContext};
use sov_modules_api::*;
use sov_prover_storage_manager::new_orphan_storage;
use sov_state::{ArrayWitness, DefaultStorageSpec, Prefix, Storage, ZkStorage};

enum Operation {
    Merge,
    Finalize,
}

impl Operation {
    fn execute<C: Context>(
        &self,
        working_set: WorkingSet<C>,
        db: C::Storage,
    ) -> StateCheckpoint<C> {
        match self {
            Operation::Merge => working_set.checkpoint(),
            Operation::Finalize => {
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
    fn execute<C: Context>(&self, mut working_set: WorkingSet<C>, db: C::Storage) -> WorkingSet<C> {
        for op in self.operations.iter() {
            working_set = op.execute(working_set, db.clone()).to_revertable()
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

fn create_state_map(
    key: u32,
    value: u32,
    working_set: &mut WorkingSet<DefaultContext>,
) -> StateMap<u32, u32> {
    let state_map = StateMap::new(Prefix::new(vec![0]));
    state_map.set(&key, &value, working_set);
    state_map
}

#[test]
fn test_state_map_with_remove() {
    let tmpdir = tempfile::tempdir().unwrap();
    let storage = new_orphan_storage(tmpdir.path()).unwrap();
    for (before_remove, after_remove) in create_storage_operations() {
        let key = 1;
        let value = 11;
        let mut working_set = WorkingSet::new(storage.clone());
        let state_map = create_state_map(key, value, &mut working_set);

        working_set = before_remove.execute(working_set, storage.clone());
        assert_eq!(state_map.remove(&key, &mut working_set).unwrap(), value);

        working_set = after_remove.execute(working_set, storage.clone());
        assert!(state_map.get(&key, &mut working_set).is_none());
    }
}

#[test]
fn test_state_map_with_delete() {
    let tmpdir = tempfile::tempdir().unwrap();
    let storage = new_orphan_storage(tmpdir.path()).unwrap();
    for (before_delete, after_delete) in create_storage_operations() {
        let key = 1;
        let value = 11;
        let mut working_set = WorkingSet::new(storage.clone());
        let state_map = create_state_map(key, value, &mut working_set);

        working_set = before_delete.execute(working_set, storage.clone());
        state_map.delete(&key, &mut working_set);

        working_set = after_delete.execute(working_set, storage.clone());
        assert!(state_map.get(&key, &mut working_set).is_none());
    }
}

fn create_state_value(value: u32, working_set: &mut WorkingSet<DefaultContext>) -> StateValue<u32> {
    let state_value = StateValue::new(Prefix::new(vec![0]));
    state_value.set(&value, working_set);
    state_value
}

#[test]
fn test_state_value_with_remove() {
    let tmpdir = tempfile::tempdir().unwrap();
    let storage = new_orphan_storage(tmpdir.path()).unwrap();
    for (before_remove, after_remove) in create_storage_operations() {
        let value = 11;
        let mut working_set = WorkingSet::new(storage.clone());
        let state_value = create_state_value(value, &mut working_set);

        working_set = before_remove.execute(working_set, storage.clone());
        assert_eq!(state_value.remove(&mut working_set).unwrap(), value);

        working_set = after_remove.execute(working_set, storage.clone());
        assert!(state_value.get(&mut working_set).is_none());
    }
}

#[test]
fn test_state_value_with_delete() {
    let tmpdir = tempfile::tempdir().unwrap();
    let storage = new_orphan_storage(tmpdir.path()).unwrap();
    for (before_delete, after_delete) in create_storage_operations() {
        let value = 11;
        let mut working_set = WorkingSet::new(storage.clone());
        let state_value = create_state_value(value, &mut working_set);

        working_set = before_delete.execute(working_set, storage.clone());
        state_value.delete(&mut working_set);

        working_set = after_delete.execute(working_set, storage.clone());
        assert!(state_value.get(&mut working_set).is_none());
    }
}

#[test]
fn test_witness_round_trip() {
    let tempdir = tempfile::tempdir().unwrap();
    let state_value = StateValue::new(Prefix::new(vec![0]));

    // Native execution
    let witness: ArrayWitness = {
        let storage = new_orphan_storage::<DefaultStorageSpec>(tempdir.path()).unwrap();
        // let storage = ProverStorage::<DefaultStorageSpec>::with_path(path).unwrap();
        let mut working_set: WorkingSet<DefaultContext> = WorkingSet::new(storage.clone());
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
        let storage = ZkStorage::<DefaultStorageSpec>::new();
        let mut working_set: WorkingSet<ZkDefaultContext> =
            WorkingSet::with_witness(storage.clone(), witness);
        state_value.set(&11, &mut working_set);
        let _ = state_value.get(&mut working_set);
        state_value.set(&22, &mut working_set);
        let (cache_log, witness) = working_set.checkpoint().freeze();

        let _ = storage
            .validate_and_commit(cache_log, &witness)
            .expect("ZK validation should succeed");
    };
}

fn create_state_vec<T: BorshDeserialize + BorshSerialize>(
    values: Vec<T>,
    working_set: &mut WorkingSet<DefaultContext>,
) -> StateVec<T> {
    let state_vec = StateVec::new(Prefix::new(vec![0]));
    state_vec.set_all(values, working_set);
    state_vec
}

#[test]
fn test_state_vec_len() {
    let tmpdir = tempfile::tempdir().unwrap();
    let storage = new_orphan_storage(tmpdir.path()).unwrap();
    for (before_len, after_len) in create_storage_operations() {
        let values = vec![11, 22, 33];
        let mut working_set = WorkingSet::new(storage.clone());
        let state_vec = create_state_vec(values.clone(), &mut working_set);

        working_set = before_len.execute(working_set, storage.clone());

        working_set = after_len.execute(working_set, storage.clone());

        assert_eq!(state_vec.len(&mut working_set), values.len());
    }
}

#[test]
fn test_state_vec_get() {
    let tmpdir = tempfile::tempdir().unwrap();
    let storage = new_orphan_storage(tmpdir.path()).unwrap();
    for (before_get, after_get) in create_storage_operations() {
        let values = vec![56, 55, 54];
        let mut working_set = WorkingSet::new(storage.clone());
        let state_vec = create_state_vec(values.clone(), &mut working_set);

        working_set = before_get.execute(working_set, storage.clone());

        let val = state_vec.get(1, &mut working_set);
        let err_val = state_vec.get_or_err(3, &mut working_set);
        assert!(val.is_some());
        assert!(err_val.is_err());

        let val = val.unwrap();
        assert_eq!(val, values.get(1).unwrap().clone());

        working_set = after_get.execute(working_set, storage.clone());
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
    let tmpdir = tempfile::tempdir().unwrap();
    let storage = new_orphan_storage(tmpdir.path()).unwrap();
    for (before_set, after_set) in create_storage_operations() {
        let values = vec![56, 55, 54];
        let mut working_set = WorkingSet::new(storage.clone());
        let state_vec = create_state_vec(values.clone(), &mut working_set);

        working_set = before_set.execute(working_set, storage.clone());
        let val = state_vec.set(1, &99, &mut working_set);
        assert!(val.is_ok());

        let val_err = state_vec.set(3, &99, &mut working_set);
        assert!(val_err.is_err());

        working_set = after_set.execute(working_set, storage.clone());

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
    let tmpdir = tempfile::tempdir().unwrap();
    let storage = new_orphan_storage(tmpdir.path()).unwrap();
    for (before_push, after_push) in create_storage_operations() {
        let values = vec![56, 55, 54];
        let mut working_set = WorkingSet::new(storage.clone());
        let state_vec = create_state_vec(values.clone(), &mut working_set);

        working_set = before_push.execute(working_set, storage.clone());

        state_vec.push(&53, &mut working_set);

        working_set = after_push.execute(working_set, storage.clone());

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
    let tmpdir = tempfile::tempdir().unwrap();
    let storage = new_orphan_storage(tmpdir.path()).unwrap();
    for (before_pop, after_pop) in create_storage_operations() {
        let values = vec![56, 55, 54];
        let mut working_set = WorkingSet::new(storage.clone());
        let state_vec = create_state_vec(values.clone(), &mut working_set);

        working_set = before_pop.execute(working_set, storage.clone());

        let popped = state_vec.pop(&mut working_set);

        assert_eq!(popped.unwrap(), 54);

        working_set = after_pop.execute(working_set, storage.clone());

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
    let tmpdir = tempfile::tempdir().unwrap();
    let storage = new_orphan_storage(tmpdir.path()).unwrap();
    for (before_set_all, after_set_all) in create_storage_operations() {
        let values = vec![56, 55, 54];
        let mut working_set = WorkingSet::new(storage.clone());
        let state_vec = create_state_vec(values.clone(), &mut working_set);

        working_set = before_set_all.execute(working_set, storage.clone());

        let new_values: Vec<u32> = vec![1];
        state_vec.set_all(new_values, &mut working_set);

        working_set = after_set_all.execute(working_set, storage.clone());

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
    let tmpdir = tempfile::tempdir().unwrap();
    let storage = new_orphan_storage(tmpdir.path()).unwrap();
    for (before_ops, after_ops) in create_storage_operations() {
        let values = vec![String::from("Hello"), String::from("World")];
        let mut working_set = WorkingSet::new(storage.clone());
        let state_vec = create_state_vec(values.clone(), &mut working_set);

        working_set = before_ops.execute(working_set, storage.clone());

        let val0 = state_vec.get(0, &mut working_set);
        let val1 = state_vec.pop(&mut working_set);
        state_vec.push(&String::from("new str"), &mut working_set);

        working_set = after_ops.execute(working_set, storage.clone());

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
