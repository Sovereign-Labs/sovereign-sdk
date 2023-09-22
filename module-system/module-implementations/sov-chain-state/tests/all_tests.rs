use sov_chain_state::{ChainState, ChainStateConfig, StateTransitionId, TransitionInProgress};
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::hooks::SlotHooks;
use sov_modules_api::{Genesis, WorkingSet};
use sov_rollup_interface::da::BlockHeaderTrait;
use sov_rollup_interface::mocks::{MockBlock, MockBlockHeader, MockDaSpec, MockValidityCond};
use sov_state::{ProverStorage, Storage};

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
        current_time: Default::default(),
    };

    // Genesis, initialize and then commit the state
    chain_state.genesis(&config, &mut working_set).unwrap();
    let (reads_writes, witness) = working_set.checkpoint().freeze();
    let genesis_root = storage.validate_and_commit(reads_writes, &witness).unwrap();

    // Computes the initial, post genesis, working set
    let mut working_set = WorkingSet::new(storage.clone());

    // Check the slot height before any changes to the state.
    let initial_height = chain_state.get_slot_height(&mut working_set);

    assert_eq!(
        initial_height, INIT_HEIGHT,
        "The initial height was not computed"
    );
    assert_eq!(
        chain_state.get_time(&mut working_set),
        Default::default(),
        "The time was not initialized to default value"
    );

    // Then simulate a transaction execution: call the begin_slot hook on a mock slot_data.
    let slot_data = MockBlock {
        header: MockBlockHeader {
            prev_hash: [0; 32].into(),
            hash: [1; 32].into(),
            height: INIT_HEIGHT,
        },
        validity_cond: MockValidityCond { is_valid: true },
        blobs: Default::default(),
    };

    chain_state.begin_slot_hook(
        &slot_data.header,
        &slot_data.validity_cond,
        &genesis_root,
        &mut working_set,
    );

    // Check that the root hash has been stored correctly
    let stored_root = chain_state.get_genesis_hash(&mut working_set).unwrap();

    assert_eq!(stored_root, genesis_root, "Genesis hashes don't match");
    assert_eq!(
        chain_state.get_time(&mut working_set),
        slot_data.header.time(),
        "The time was not updated in the hook"
    );

    // Check that the slot height has been updated
    let new_height_storage = chain_state.get_slot_height(&mut working_set);

    assert_eq!(
        new_height_storage,
        INIT_HEIGHT + 1,
        "The new height did not update"
    );

    // Check that the new state transition is being stored
    let new_tx_in_progress: TransitionInProgress<MockDaSpec> = chain_state
        .get_in_progress_transition(&mut working_set)
        .unwrap();

    assert_eq!(
        new_tx_in_progress,
        TransitionInProgress::<MockDaSpec>::new(
            [1; 32].into(),
            MockValidityCond { is_valid: true }
        ),
        "The new transition has not been correctly stored"
    );

    // We now commit the new state (which updates the root hash)
    let (reads_writes, witness) = working_set.checkpoint().freeze();
    let new_root_hash = storage.validate_and_commit(reads_writes, &witness).unwrap();

    // Computes the new working set
    let mut working_set = WorkingSet::new(storage);

    // And we simulate a new slot application by calling the `begin_slot` hook.
    let new_slot_data = MockBlock {
        header: MockBlockHeader {
            prev_hash: [1; 32].into(),
            hash: [2; 32].into(),
            height: INIT_HEIGHT,
        },
        validity_cond: MockValidityCond { is_valid: false },
        blobs: Default::default(),
    };

    chain_state.begin_slot_hook(
        &new_slot_data.header,
        &new_slot_data.validity_cond,
        &new_root_hash,
        &mut working_set,
    );

    // Check that the slot height has been updated correctly
    let new_height_storage = chain_state.get_slot_height(&mut working_set);
    assert_eq!(
        new_height_storage,
        INIT_HEIGHT + 2,
        "The new height did not update"
    );
    assert_eq!(
        chain_state.get_time(&mut working_set),
        new_slot_data.header.time(),
        "The time was not updated in the hook"
    );

    // Check the transition in progress
    let new_tx_in_progress: TransitionInProgress<MockDaSpec> = chain_state
        .get_in_progress_transition(&mut working_set)
        .unwrap();

    assert_eq!(
        new_tx_in_progress,
        TransitionInProgress::<MockDaSpec>::new(
            [2; 32].into(),
            MockValidityCond { is_valid: false }
        ),
        "The new transition has not been correctly stored"
    );

    // Check the transition stored
    let last_tx_stored: StateTransitionId<MockDaSpec, _> = chain_state
        .get_historical_transitions(INIT_HEIGHT + 1, &mut working_set)
        .unwrap();

    assert_eq!(
        last_tx_stored,
        StateTransitionId::new(
            [1; 32].into(),
            new_root_hash,
            MockValidityCond { is_valid: true }
        )
    );

    assert_ne!(
        chain_state.get_time(&mut working_set),
        Default::default(),
        "The time must be updated"
    );
}
