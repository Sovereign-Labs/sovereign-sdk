use std::convert::Infallible;

use capabilities::mocks::MockKernel;
use sov_modules_api::*;
use sov_state::{
    ArrayWitness, BorshCodec, Prefix, ProverStorage, StateAccesses, Storage, ZkStorage,
};
use unwrap_infallible::UnwrapInfallible;

use crate::state_tests::*;

pub trait StateThing {
    type Value: core::fmt::Debug + Eq + PartialEq;

    /// Write itself to the underlying infallible state accessor
    fn create(state: &mut impl InfallibleStateAccessor) -> Self;

    /// Gets value from the underlying infallible state accessor
    fn value(&self, state: &mut impl InfallibleStateAccessor) -> Self::Value;

    /// Changes itself in the underlying infallible state accessor
    fn change(&mut self, state: &mut impl InfallibleStateAccessor);
}

pub enum Condition {
    Checkpoint,
    Revert,
}

pub struct StateValueSet(StateValue<u32>);

impl StateThing for StateValueSet {
    type Value = u32;

    fn create(state: &mut impl InfallibleStateAccessor) -> Self {
        let mut state_value = StateValue::with_codec(Prefix::new(vec![0]), BorshCodec);
        state_value.set(&10, state).unwrap_infallible();
        StateValueSet(state_value)
    }

    fn value(&self, state: &mut impl InfallibleStateAccessor) -> Self::Value {
        self.0
            .get(state)
            .unwrap_infallible()
            .expect("Value wasn't set")
    }

    fn change(&mut self, state: &mut impl InfallibleStateAccessor) {
        let mut value = self.value(state);
        value += 1;
        self.0.set(&value, state).unwrap_infallible();
    }
}

pub struct StateVecSet(StateVec<u32>);

impl StateThing for StateVecSet {
    type Value = Vec<u32>;

    fn create(state: &mut impl InfallibleStateAccessor) -> Self {
        let mut state_vec = StateVec::with_codec(Prefix::new(vec![0]), BorshCodec);
        state_vec
            .set_all(vec![10, 20, 30, 40, 50, 60], state)
            .unwrap_infallible();
        StateVecSet(state_vec)
    }

    fn value(&self, state: &mut impl InfallibleStateAccessor) -> Self::Value {
        self.0.collect_infallible(state)
    }

    fn change(&mut self, state: &mut impl InfallibleStateAccessor) {
        let mut value = self.value(state);
        for v in value.iter_mut() {
            *v += 1;
        }
        self.0.set_all(value, state).unwrap_infallible();
    }
}

pub struct StateVecPush(StateVec<u32>);

impl StateThing for StateVecPush {
    type Value = Vec<u32>;

    fn create(state: &mut impl InfallibleStateAccessor) -> Self {
        let mut state_vec = StateVec::with_codec(Prefix::new(vec![0]), BorshCodec);
        state_vec.set_all(vec![10], state).unwrap_infallible();
        StateVecPush(state_vec)
    }

    fn value(&self, state: &mut impl InfallibleStateAccessor) -> Self::Value {
        self.0.collect_infallible(state)
    }

    fn change(&mut self, state: &mut impl InfallibleStateAccessor) {
        let value = self
            .0
            .get(0, state)
            .unwrap_infallible()
            .expect("Value wasn't set");
        self.0.push(&(value + 1), state).unwrap_infallible();
    }
}

pub struct StateVecRemove(StateVec<u32>);

impl StateThing for StateVecRemove {
    type Value = Vec<u32>;

    fn create(state: &mut impl InfallibleStateAccessor) -> Self {
        let mut state_vec = StateVec::with_codec(Prefix::new(vec![0]), BorshCodec);
        state_vec
            .set_all(vec![3u32; 100], state)
            .unwrap_infallible();
        StateVecRemove(state_vec)
    }

    fn value(&self, state: &mut impl InfallibleStateAccessor) -> Self::Value {
        self.0.collect_infallible(state)
    }

    fn change(&mut self, state: &mut impl InfallibleStateAccessor) {
        self.0.pop(state).unwrap_infallible();
    }
}

impl Condition {
    fn replace_working_set<S: Spec>(&self, working_set: WorkingSet<S>) -> WorkingSet<S> {
        match self {
            Condition::Checkpoint => {
                let (scratchpad, _tx_consumption, _events) = working_set.finalize();
                let checkpoint = scratchpad.commit();
                checkpoint.to_working_set_unmetered()
            }
            Condition::Revert => {
                let (tx_scratchpad, _tx_consumption) = working_set.revert();
                let checkpoint = tx_scratchpad.revert();
                checkpoint.to_working_set_unmetered()
            }
        }
    }

    fn process_thing<S: Spec, St: StateThing>(
        &self,
        thing: &mut St,
        mut working_set: WorkingSet<S>,
    ) -> WorkingSet<S> {
        let value_before = thing.value(&mut working_set.to_unmetered());
        thing.change(&mut working_set.to_unmetered());
        working_set = self.replace_working_set(working_set);
        let value_after = thing.value(&mut working_set.to_unmetered());
        match self {
            Condition::Checkpoint => {
                assert_ne!(
                    value_before, value_after,
                    "Value hasn't been changed after `.checkpoint()`"
                );
            }
            Condition::Revert => {
                assert_eq!(
                    value_before, value_after,
                    "Value has changed after `.revert()`"
                );
            }
        }
        working_set
    }
}

const CONDITIONS: [Condition; 8] = [
    Condition::Checkpoint,
    Condition::Revert,
    Condition::Checkpoint,
    Condition::Revert,
    Condition::Checkpoint,
    Condition::Revert,
    Condition::Revert,
    Condition::Checkpoint,
];

/// Creates thing and checks it with all condition combinations
pub fn test_state_thing<S: Spec<Storage = ProverStorage<StorageSpec>>, St: StateThing>(
    conditions: &[Condition],
) {
    let simple_storage_manager = SimpleStorageManager::new();
    let storage: ProverStorage<StorageSpec> = simple_storage_manager.create_storage();
    let mut state = StateCheckpoint::<S>::new(storage, &MockKernel::<S>::default());
    let mut thing = St::create(&mut state);
    let mut working_set = state.to_working_set_unmetered();

    for condition in conditions {
        working_set = condition.process_thing(&mut thing, working_set);
    }
}

#[test]
fn test_state_value_set() {
    test_state_thing::<TestSpec, StateValueSet>(&CONDITIONS[..]);
}

#[test]
fn test_state_vec_set() {
    test_state_thing::<TestSpec, StateVecSet>(&CONDITIONS[..]);
}

#[test]
fn test_state_vec_push() {
    test_state_thing::<TestSpec, StateVecPush>(&CONDITIONS[..]);
}

#[test]
fn test_state_vec_remove() {
    test_state_thing::<TestSpec, StateVecRemove>(&CONDITIONS[..]);
}

#[test]
fn test_witness_round_trip() -> Result<(), Infallible> {
    let mut storage_manager = SimpleStorageManager::<StorageSpec>::new();

    let mut state_value = StateValue::with_codec(Prefix::new(vec![0]), BorshCodec);

    // Native execution
    let (witness, root) = {
        // Simulate genesis.
        // First native call to `validate_and_materialize` is during genesis,
        // when witness is not populated.
        let storage = storage_manager.create_storage();
        let (root, genesis_change_set) = validate_and_materialize(
            storage,
            StateAccesses {
                user: Default::default(),
                kernel: Default::default(),
            },
            &ArrayWitness::default(),
            <<TestSpec as Spec>::Storage as Storage>::PRE_GENESIS_ROOT,
        )
        .expect("Native jmt validation should succeed");
        storage_manager.commit(genesis_change_set);
        // Actual
        let mut mock_kernel = MockKernel::<TestSpec>::default();
        mock_kernel.increase_heights();
        let storage = storage_manager.create_storage();
        let mut state: StateCheckpoint<TestSpec> =
            StateCheckpoint::new(storage.clone(), &mock_kernel);
        state_value.set(&11, &mut state)?;
        let _ = state_value.get(&mut state);
        state_value.set(&22, &mut state)?;
        let (cache_log, _, witness) = state.freeze();

        let _ = validate_and_materialize(storage, cache_log, &witness, root)
            .expect("Native jmt validation should succeed");
        (witness, root)
    };

    {
        let storage = ZkStorage::<StorageSpec>::new();
        let mut state_checkpoint: StateCheckpoint<Zk> =
            StateCheckpoint::with_witness(storage.clone(), witness, &MockKernel::<Zk>::default());
        state_value.set(&11, &mut state_checkpoint)?;
        let _ = state_value.get(&mut state_checkpoint);
        state_value.set(&22, &mut state_checkpoint)?;
        let (cache_log, _, witness) = state_checkpoint.freeze();

        let _ = validate_and_materialize(storage, cache_log, &witness, root)
            .expect("ZK validation should succeed");
    };

    Ok(())
}

/// Test that the borrow API returns the expected value and allows other non-mutating operations
/// for a `StateValue`
#[test]
fn test_borrow_and_get_state_value() {
    let storage_manager = SimpleStorageManager::<StorageSpec>::new();
    let storage = storage_manager.create_storage();
    let mut state = StateCheckpoint::<TestSpec>::new(storage, &MockKernel::<TestSpec>::default());
    let mut state_value = StateValue::with_codec(Prefix::new(vec![0]), BorshCodec);

    let val = state_value.borrow(&mut state).unwrap_infallible();
    assert!(val.is_none());
    state_value.set(&11, &mut state).unwrap_infallible();

    // Borrow the value twice and check that they're equal
    let val1 = state_value.borrow(&mut state).unwrap_infallible().unwrap();
    let val2 = state_value.borrow(&mut state).unwrap_infallible().unwrap();
    assert_eq!(*val1, 11);
    assert_eq!(*val1, *val2);

    // Check that you can use the `get api` while borrowed
    let val3 = state_value.get(&mut state).unwrap_infallible().unwrap();
    assert_eq!(val3, *val1);
}

/// Test that the borrow API returns the expected value and allows other non-mutating operations
/// for a `StateValue`
#[test]
fn test_borrow_and_save_state_value() {
    let storage_manager = SimpleStorageManager::<StorageSpec>::new();
    let storage = storage_manager.create_storage();
    let mut state = StateCheckpoint::<TestSpec>::new(storage, &MockKernel::<TestSpec>::default());
    let mut state_value = StateValue::<i32>::with_codec(Prefix::new(vec![0]), BorshCodec);

    let val = state_value.borrow_mut(&mut state).unwrap_infallible();
    assert!(val.is_none());
    state_value.set(&11, &mut state).unwrap_infallible();

    // Borrow the value and mutate it
    let mut val1 = state_value
        .borrow_mut(&mut state)
        .unwrap_infallible()
        .unwrap();
    assert_eq!(*val1, 11);
    *val1 += 1;
    val1.save(&mut state).unwrap_infallible();

    // check that the value was mutated
    let val2 = state_value.get(&mut state).unwrap_infallible().unwrap();
    assert_eq!(val2, 12);

    // Borrow the value and mutate it
    let val = state_value
        .borrow_mut(&mut state)
        .unwrap_infallible()
        .unwrap();
    assert_eq!(*val, 12);
    val.delete(&mut state).unwrap_infallible();

    // check that the value was mutated
    let val2 = state_value.get(&mut state).unwrap_infallible();
    assert!(val2.is_none());
}

/// Test that the borrow API returns the expected value and allows other non-mutating operations
/// for a `StateMap`
#[test]
fn test_borrow_and_get_state_map() {
    let storage_manager = SimpleStorageManager::<StorageSpec>::new();
    let storage = storage_manager.create_storage();
    let mut state = StateCheckpoint::<TestSpec>::new(storage, &MockKernel::<TestSpec>::default());
    let mut state_map = StateMap::with_codec(Prefix::new(vec![0]), BorshCodec);

    let val = state_map.borrow(&0, &mut state).unwrap_infallible();
    assert!(val.is_none());
    state_map.set(&0, &11, &mut state).unwrap_infallible();

    // Borrow the value twice and check that they're equal
    let val1 = state_map
        .borrow(&0, &mut state)
        .unwrap_infallible()
        .unwrap();
    let val2 = state_map
        .borrow(&0, &mut state)
        .unwrap_infallible()
        .unwrap();
    assert_eq!(*val1, 11);
    assert_eq!(*val1, *val2);

    // Check that you can use the `get api` while borrowed
    let val3 = state_map.get(&0, &mut state).unwrap_infallible().unwrap();
    assert_eq!(val3, *val1);
}

/// Test that the borrow API returns the expected value and allows other non-mutating operations
/// for a `StateValue`
#[test]
fn test_borrow_and_save_state_map() {
    let storage_manager = SimpleStorageManager::<StorageSpec>::new();
    let storage = storage_manager.create_storage();
    let mut state = StateCheckpoint::<TestSpec>::new(storage, &MockKernel::<TestSpec>::default());
    let mut state_map = StateMap::with_codec(Prefix::new(vec![0]), BorshCodec);

    let val = state_map.borrow(&0, &mut state).unwrap_infallible();
    assert!(val.is_none());
    state_map.set(&0, &11, &mut state).unwrap_infallible();

    // Borrow the value and mutate it
    let mut val1 = state_map
        .borrow_mut(&0, &mut state)
        .unwrap_infallible()
        .unwrap();
    assert_eq!(*val1, 11);
    *val1 += 1;
    val1.save(&mut state).unwrap_infallible();

    // check that the value was mutated
    let val2 = state_map.get(&0, &mut state).unwrap_infallible().unwrap();
    assert_eq!(val2, 12);

    // Borrow the value and mutate it
    let val = state_map
        .borrow_mut(&0, &mut state)
        .unwrap_infallible()
        .unwrap();
    assert_eq!(*val, 12);
    val.delete(&mut state).unwrap_infallible();

    // check that the value was mutated
    let val2 = state_map.get(&0, &mut state).unwrap_infallible();
    assert!(val2.is_none());
}
