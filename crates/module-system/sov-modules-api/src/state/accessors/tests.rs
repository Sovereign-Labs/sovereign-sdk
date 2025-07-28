use sov_mock_zkvm::MockZkvm;
use sov_modules_api::execution_mode;
use sov_state::{
    AccessSize, ArrayWitness, BorshCodec, IsValueCached, Namespace, OrderedReadsAndWrites, Prefix,
    StateAccesses, Storage, ZkStorage,
};
use sov_test_utils::storage::SimpleStorageManager;
use sov_test_utils::{validate_and_materialize, MockDaSpec, TestStorageSpec};

use super::seal::UniversalStateAccessor;
use crate::capabilities::mocks::MockKernel;
use crate::{Spec, StateCheckpoint, StateValue};

type Native =
    crate::default_spec::DefaultSpec<MockDaSpec, MockZkvm, MockZkvm, execution_mode::Native>;

type Zk = crate::default_spec::DefaultSpec<MockDaSpec, MockZkvm, MockZkvm, execution_mode::Zk>;

const PRE_SET_VAL_ID_1: u8 = 0;
const PRE_SET_VAL_ID_2: u8 = 1;
const VAL_ID_1: u8 = 2;
const NAMESPACE: Namespace = Namespace::User;

fn create_storage_manager(
    initial_values: Vec<(Vec<u8>, u64)>,
) -> (
    SimpleStorageManager<TestStorageSpec>,
    <<Native as Spec>::Storage as Storage>::Root,
) /*ProverStorage<DefaultStorageSpec<sha2::Sha256>>*/
{
    let mut storage_manager = SimpleStorageManager::new();
    let storage = storage_manager.create_storage();

    let (root, genesis_change_set) = validate_and_materialize(
        storage,
        StateAccesses {
            user: OrderedReadsAndWrites {
                ordered_reads: Default::default(),
                ordered_writes: initial_values
                    .into_iter()
                    .map(|(k, v)| {
                        let state_value = StateValue::<u64>::with_codec(Prefix::new(k), BorshCodec);
                        (state_value.slot_key(), Some(state_value.slot_value(&v)))
                    })
                    .collect(),
            },
            kernel: Default::default(),
        },
        &ArrayWitness::default(),
        <Native as Spec>::Storage::PRE_GENESIS_ROOT,
    )
    .expect("Native jmt validation should succeed");
    storage_manager.commit(genesis_change_set);
    (storage_manager, root)
}

#[test]
fn test_witness_generation() {
    // Run the test with Native storage and create the witness.
    let (witness, root) = {
        let (manager, root) = create_storage_manager(vec![
            (vec![PRE_SET_VAL_ID_1], 22),
            (vec![PRE_SET_VAL_ID_2], 99),
        ]);
        let storage = manager.create_storage();

        let mut state = StateCheckpoint::new(storage.clone(), &MockKernel::<Native>::default());

        test_values(&mut state);
        let (cache_log, _, witness) = state.freeze();

        let _ = validate_and_materialize(storage, cache_log, &witness, root)
            .expect("Native jmt validation should succeed");
        (witness, root)
    };

    // Run the test with Zk storage and consume the witness.
    {
        let storage = ZkStorage::new();
        let mut state =
            StateCheckpoint::with_witness(storage.clone(), witness, &MockKernel::<Zk>::default());

        test_values(&mut state);

        let (cache_log, _, witness) = state.freeze();

        let _ = validate_and_materialize(storage, cache_log, &witness, root)
            .expect("ZK validation should succeed");
    }
}

fn test_values<S: Spec>(state: &mut StateCheckpoint<S>) {
    // Test overriding empty value.
    {
        let mut state_value =
            StateValue::<u64>::with_codec(Prefix::new(vec![VAL_ID_1]), BorshCodec);
        let is_cached = state.is_value_cached(NAMESPACE, &state_value.slot_key());
        assert_eq!(is_cached, IsValueCached::No);

        let maybe_size = state.get_size(NAMESPACE, &state_value.slot_key());
        assert!(maybe_size.is_none());

        let maybe_get = state_value.get(state).unwrap();
        assert!(maybe_get.is_none());

        state_value.set(&77, state).unwrap();
        let is_cached = state.is_value_cached(NAMESPACE, &state_value.slot_key());
        assert_eq!(is_cached, IsValueCached::Yes(AccessSize::Write(8)));

        let maybe_size = state.get_size(NAMESPACE, &state_value.slot_key());
        assert_eq!(maybe_size, Some(8));

        let maybe_get = state_value.get(state).unwrap();
        assert_eq!(maybe_get, Some(77));
    }

    // Test overriding pre-set value.
    {
        let mut state_value =
            StateValue::<u64>::with_codec(Prefix::new(vec![PRE_SET_VAL_ID_1]), BorshCodec);

        let is_cached = state.is_value_cached(NAMESPACE, &state_value.slot_key());
        assert_eq!(is_cached, IsValueCached::No);

        let maybe_size = state.get_size(NAMESPACE, &state_value.slot_key());
        assert_eq!(maybe_size, Some(8));

        let is_cached = state.is_value_cached(NAMESPACE, &state_value.slot_key());
        assert_eq!(is_cached, IsValueCached::Yes(AccessSize::Read(8)));

        let maybe_get = state_value.get(state).unwrap();
        assert_eq!(maybe_get, Some(22));

        state_value.set(&66, state).unwrap();

        let maybe_get = state_value.get(state).unwrap();
        assert_eq!(maybe_get, Some(66));
    }

    // Test scenario where we read first and then get the size.
    {
        let state_value =
            StateValue::<u64>::with_codec(Prefix::new(vec![PRE_SET_VAL_ID_2]), BorshCodec);

        let is_cached = state.is_value_cached(NAMESPACE, &state_value.slot_key());
        assert_eq!(is_cached, IsValueCached::No);

        let maybe_get = state_value.get(state).unwrap();
        assert_eq!(maybe_get, Some(99));

        let is_cached = state.is_value_cached(NAMESPACE, &state_value.slot_key());
        assert_eq!(is_cached, IsValueCached::Yes(AccessSize::Read(8)));

        let maybe_size = state.get_size(NAMESPACE, &state_value.slot_key());
        assert_eq!(maybe_size, Some(8));
    }
}

#[test]
fn test_discard_tx_cache() {
    let (manager, _) = create_storage_manager(vec![
        (vec![PRE_SET_VAL_ID_1], 22),
        (vec![PRE_SET_VAL_ID_2], 99),
    ]);
    let storage = manager.create_storage();

    let state_value_to_read =
        StateValue::<u64>::with_codec(Prefix::new(vec![PRE_SET_VAL_ID_1]), BorshCodec);

    let mut state_value_to_set =
        StateValue::<u64>::with_codec(Prefix::new(vec![VAL_ID_1]), BorshCodec);

    // Not discarded values are present after the freeze.
    {
        let mut state = StateCheckpoint::new(storage.clone(), &MockKernel::<Native>::default());

        let _ = state_value_to_read.get(&mut state).unwrap();
        state_value_to_set.set(&99, &mut state).unwrap();

        let (state_accesses, _, _) = state.freeze();
        let ordered_reads = state_accesses.user.ordered_reads;
        let ordered_writes = state_accesses.user.ordered_writes;

        assert!(!ordered_reads.is_empty());
        assert!(!ordered_writes.is_empty());
    }

    // Discarded values are empty after the freeze.
    {
        let mut state = StateCheckpoint::new(storage.clone(), &MockKernel::<Native>::default());
        let _ = state_value_to_read.get(&mut state).unwrap();
        state_value_to_set.set(&99, &mut state).unwrap();

        state.discard_revertable_storage_cache();
        let (state_accesses, _, _) = state.freeze();
        let ordered_reads = state_accesses.user.ordered_reads;
        let ordered_writes = state_accesses.user.ordered_writes;

        assert!(ordered_reads.is_empty());
        assert!(ordered_writes.is_empty());
    }
}
