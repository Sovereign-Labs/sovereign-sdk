use sov_chain_state::{ChainState, ChainStateConfig, StateTransitionId, TransitionInProgress};
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::hooks::SlotHooks;
use sov_modules_api::Genesis;
use sov_rollup_interface::mocks::{
    MockBlock, MockBlockHeader, MockDaSpec, MockHash, MockValidityCond,
};
use sov_state::{ProverStorage, Storage, WorkingSet};

/// This simply tests that the chain_state reacts properly with the invocation of the `begin_slot`
/// hook. For more complete integration tests, feel free to have a look at the integration tests folder.
#[test]
fn test_simple_chain_state() {
    // The initial height can be any value.
    const INIT_HEIGHT: u64 = 10;
    // Initialize the module.
    let tmpdir = tempfile::tempdir().unwrap();

    let storage: ProverStorage<sov_state::DefaultStorageSpec> =
        ProverStorage::with_path(tmpdir.path()).unwrap();

    let mut working_set = WorkingSet::new(storage.clone());

    let chain_state = ChainState::<DefaultContext, MockDaSpec>::default();
    let config = ChainStateConfig {
        initial_slot_height: INIT_HEIGHT,
    };

    // Genesis, initialize and then commit the state
    chain_state.genesis(&config, &mut working_set).unwrap();
    let (reads_writes, witness) = working_set.checkpoint().freeze();
    storage.validate_and_commit(reads_writes, &witness).unwrap();

    // Computes the initial, post genesis, working set
    let mut working_set = WorkingSet::new(storage.clone());

    // Check the slot height before any changes to the state.
    let initial_height = chain_state.get_slot_height(&mut working_set);

    assert_eq!(
        initial_height, INIT_HEIGHT,
        "The initial height was not computed"
    );

    // Then simulate a transaction execution: call the begin_slot hook on a mock slot_data.
    let slot_data = MockBlock {
        curr_hash: [1; 32],
        header: MockBlockHeader {
            prev_hash: MockHash([0; 32]),
        },
        height: INIT_HEIGHT,
        validity_cond: MockValidityCond { is_valid: true },
        blobs: Default::default(),
    };

    chain_state.begin_slot_hook(&slot_data, &mut working_set);

    // Check that the root hash has been stored correctly
    let stored_root: [u8; 32] = chain_state.get_genesis_hash(&mut working_set).unwrap();
    let init_root_hash = storage.get_state_root(&Default::default()).unwrap();

    assert_eq!(stored_root, init_root_hash, "Genesis hashes don't match");

    // Check that the slot height has been updated
    let new_height_storage = chain_state.get_slot_height(&mut working_set);

    assert_eq!(
        new_height_storage,
        INIT_HEIGHT + 1,
        "The new height did not update"
    );

    // Check that the new state transition is being stored
    let new_tx_in_progress: TransitionInProgress<MockValidityCond> = chain_state
        .get_in_progress_transition(&mut working_set)
        .unwrap();

    assert_eq!(
        new_tx_in_progress,
        TransitionInProgress::<MockValidityCond>::new([1; 32], MockValidityCond { is_valid: true }),
        "The new transition has not been correctly stored"
    );

    // We now commit the new state (which updates the root hash)
    let (reads_writes, witness) = working_set.checkpoint().freeze();
    storage.validate_and_commit(reads_writes, &witness).unwrap();
    let new_root_hash = storage.get_state_root(&Default::default());

    // Computes the new working set
    let mut working_set = WorkingSet::new(storage);

    // And we simulate a new slot application by calling the `begin_slot` hook.
    let new_slot_data = MockBlock {
        curr_hash: [2; 32],
        header: MockBlockHeader {
            prev_hash: MockHash([1; 32]),
        },
        height: INIT_HEIGHT,
        validity_cond: MockValidityCond { is_valid: false },
        blobs: Default::default(),
    };

    chain_state.begin_slot_hook(&new_slot_data, &mut working_set);

    // Check that the slot height has been updated correctly
    let new_height_storage = chain_state.get_slot_height(&mut working_set);
    assert_eq!(
        new_height_storage,
        INIT_HEIGHT + 2,
        "The new height did not update"
    );

    // Check the transition in progress
    let new_tx_in_progress: TransitionInProgress<MockValidityCond> = chain_state
        .get_in_progress_transition(&mut working_set)
        .unwrap();

    assert_eq!(
        new_tx_in_progress,
        TransitionInProgress::<MockValidityCond>::new(
            [2; 32],
            MockValidityCond { is_valid: false }
        ),
        "The new transition has not been correctly stored"
    );

    // Check the transition stored
    let last_tx_stored: StateTransitionId<MockValidityCond> = chain_state
        .get_historical_transitions(INIT_HEIGHT + 1, &mut working_set)
        .unwrap();

    assert_eq!(
        last_tx_stored,
        StateTransitionId::new(
            [1; 32],
            new_root_hash.unwrap(),
            MockValidityCond { is_valid: true }
        )
    );
}
