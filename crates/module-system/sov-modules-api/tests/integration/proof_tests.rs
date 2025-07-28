use std::sync::Arc;

use capabilities::mocks::MockKernel;
use capabilities::RollupHeight;
use sov_modules_api::*;
use sov_state::{BorshCodec, Prefix, Storage, StorageProof};
use sov_test_utils::storage::SimpleStorageManager;
use sov_test_utils::validate_and_materialize;
use unwrap_infallible::UnwrapInfallible;

type S = sov_test_utils::TestSpec;

#[allow(clippy::type_complexity)]
fn make_user_map_proof(
    key: u32,
    value: u32,
) -> (
    <<S as Spec>::Storage as Storage>::Root,
    StorageProof<<<S as Spec>::Storage as Storage>::Proof>,
    StateMap<u32, u32>,
) {
    let kernel = MockKernel::<S>::default();
    let mut storage_manager = SimpleStorageManager::new();
    let storage = storage_manager.create_storage();
    let mut state = StateCheckpoint::<S>::new(storage.clone(), &kernel);
    let mut map = StateMap::with_codec(Prefix::new(vec![0]), BorshCodec);
    map.set(&key, &value, &mut state).unwrap_infallible();

    let (cache_log, _, witness) = state.freeze();

    let (root, change_set) = validate_and_materialize(
        storage,
        cache_log,
        &witness,
        <<S as Spec>::Storage as Storage>::PRE_GENESIS_ROOT,
    )
    .expect("Native jmt validation should succeed");
    storage_manager.commit(change_set);
    let storage = storage_manager.create_storage();

    let state_checkpoint = StateCheckpoint::new(storage, &kernel);
    let mut state = ApiStateAccessor::new(&state_checkpoint, Arc::new(kernel));

    let proof = map.get_with_proof(&1, &mut state).unwrap();
    (root, proof, map)
}

#[allow(clippy::type_complexity)]
fn make_user_value_proof(
    value: u32,
) -> (
    <<S as Spec>::Storage as Storage>::Root,
    StorageProof<<<S as Spec>::Storage as Storage>::Proof>,
    StateValue<u32>,
) {
    let kernel = MockKernel::<S>::default();
    let mut storage_manager = SimpleStorageManager::new();
    let storage = storage_manager.create_storage();
    let mut state = StateCheckpoint::<S>::new(storage.clone(), &MockKernel::<S>::default());
    let mut state_val = StateValue::with_codec(Prefix::new(vec![0]), BorshCodec);
    state_val.set(&value, &mut state).unwrap_infallible();

    let (cache_log, _, witness) = state.freeze();

    let (root, change_set) = validate_and_materialize(
        storage,
        cache_log,
        &witness,
        <S as Spec>::Storage::PRE_GENESIS_ROOT,
    )
    .expect("Native jmt validation should succeed");
    storage_manager.commit(change_set);
    let storage = storage_manager.create_storage();

    let state_checkpoint = StateCheckpoint::new(storage, &kernel);
    let mut state = ApiStateAccessor::new(&state_checkpoint, Arc::new(kernel));

    let proof = state_val.get_with_proof(&mut state).unwrap();
    (root, proof, state_val)
}

mod map {
    use sov_state::{Prefix, ProvableNamespace, SlotKey, SlotValue};

    use super::{make_user_map_proof, S};

    #[test]
    fn test_state_proof_roundtrip() {
        let (root, proof, map) = make_user_map_proof(1, 2);
        let (key, val) = map.verify_proof::<S>(root, proof).unwrap();
        assert_eq!(key, 1);
        assert_eq!(val, Some(2));
    }

    #[test]
    fn test_state_proof_wrong_namespace() {
        let (root, mut proof, map) = make_user_map_proof(1, 2);
        proof.namespace = ProvableNamespace::Kernel;
        assert!(map.verify_proof::<S>(root, proof).is_err());
    }

    #[test]
    fn test_state_proof_wrong_key() {
        let (root, mut proof, map) = make_user_map_proof(1, 2);
        proof.key = SlotKey::new(&Prefix::new(b"wrong_prefix".to_vec()), &1, map.codec());
        assert!(map.verify_proof::<S>(root, proof).is_err());
    }

    #[test]
    fn test_state_proof_missing_value() {
        let (root, mut proof, map) = make_user_map_proof(1, 2);
        proof.value = None;
        assert!(map.verify_proof::<S>(root, proof).is_err());
    }

    #[test]
    fn test_state_proof_wrong_value() {
        let (root, mut proof, map) = make_user_map_proof(1, 2);
        proof.value = Some(SlotValue::new(&3, map.codec()));
        assert!(map.verify_proof::<S>(root, proof).is_err());
    }
}

mod value {
    use sov_state::{Prefix, ProvableNamespace, SlotKey, SlotValue};

    use super::{make_user_value_proof, S};

    #[test]
    fn test_state_proof_roundtrip() {
        let (root, proof, map) = make_user_value_proof(1);
        let val = map.verify_proof::<S>(root, proof).unwrap();
        assert_eq!(val, Some(1));
    }

    #[test]
    fn test_state_proof_wrong_namespace() {
        let (root, mut proof, map) = make_user_value_proof(1);
        proof.namespace = ProvableNamespace::Kernel;
        assert!(map.verify_proof::<S>(root, proof).is_err());
    }

    #[test]
    fn test_state_proof_wrong_key() {
        let (root, mut proof, map) = make_user_value_proof(1);
        proof.key = SlotKey::new(&Prefix::new(b"wrong_prefix".to_vec()), &1, map.codec());
        assert!(map.verify_proof::<S>(root, proof).is_err());
    }

    #[test]
    fn test_state_proof_missing_value() {
        let (root, mut proof, map) = make_user_value_proof(1);
        proof.value = None;
        assert!(map.verify_proof::<S>(root, proof).is_err());
    }

    #[test]
    fn test_state_proof_wrong_value() {
        let (root, mut proof, map) = make_user_value_proof(1);
        proof.value = Some(SlotValue::new(&3, map.codec()));
        assert!(map.verify_proof::<S>(root, proof).is_err());
    }
}

#[test]
fn test_archival_proof_gen() {
    let mut kernel = MockKernel::<S>::default();
    let mut storage_manager = SimpleStorageManager::new();
    let mut state_val = StateValue::with_codec(Prefix::new(vec![0]), BorshCodec);

    const NUM_ITER: u64 = 10;

    // Update the state value and calculate a new root for each iteration
    let mut roots = vec![];
    let mut current_root = <S as Spec>::Storage::PRE_GENESIS_ROOT;
    for iter in 0..NUM_ITER {
        let storage = storage_manager.create_storage();

        // We need to write to the version zero, hence the condition to ensure we have kernel heights matching version numbers.
        if iter > 0 {
            kernel.increase_heights();
        }

        let mut state = StateCheckpoint::<S>::new(storage.clone(), &kernel);

        if iter % 2 == 0 {
            state_val.set(&iter, &mut state).unwrap_infallible();
        } else {
            state_val.delete(&mut state).unwrap_infallible();
        }

        let (cache_log, _, witness) = state.freeze();

        let (root, change_set) =
            validate_and_materialize(storage, cache_log, &witness, current_root)
                .expect("Native jmt validation should succeed");
        current_root = root;

        storage_manager.commit(change_set);

        roots.push(root);
    }

    let storage = storage_manager.create_storage();
    // Generate a proof at each archival state and validate it against the root
    let state_checkpoint = StateCheckpoint::new(storage.clone(), &kernel);
    let mut api_state_accessor = ApiStateAccessor::new(&state_checkpoint, Arc::new(kernel));
    for iter in 0..NUM_ITER {
        let mut archival_accessor = api_state_accessor
            .get_archival_state(RollupHeight::new(iter))
            .unwrap();
        let proof = state_val.get_with_proof(&mut archival_accessor).unwrap();
        let value = state_val
            .verify_proof::<S>(roots[iter as usize], proof)
            .unwrap();
        if iter % 2 == 0 {
            assert_eq!(value, Some(iter));
        } else {
            assert!(value.is_none());
        }
    }

    // Check that the default working_set use the latest state for archival proof generation
    let proof = state_val.get_with_proof(&mut api_state_accessor).unwrap();
    let final_value = state_val.verify_proof::<S>(roots[9], proof).unwrap();
    assert_eq!(final_value, None);
}
