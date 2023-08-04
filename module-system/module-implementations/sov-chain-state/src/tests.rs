use borsh::BorshDeserialize;
use sov_data_generators::value_setter_data::ValueSetterMessages;
use sov_data_generators::{has_tx_events, new_test_blob_from_batch, MessageGenerator};
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::Spec;
use sov_modules_stf_template::{AppTemplate, SequencerOutcome};
use sov_rollup_interface::mocks::{
    MockZkvm, TestBlob, TestBlock, TestBlockHeader, TestHash, TestValidityCond,
};
use sov_rollup_interface::stf::StateTransitionFunction;
use sov_state::storage::StorageKey;
use sov_state::{ProverStorage, SingletonKey};

use crate::tests_helpers::{create_demo_genesis_config, TestRuntime};
use crate::TransitionInProgress;

type C = DefaultContext;

/// This test generates a new mock rollup having a simple value setter module
/// with an associated chain state, and checks that the height, the genesis hash
/// and the state transitions are correctly stored and updated.
#[test]
fn test_simple_value_setter() {
    // Build an app template with the module configurations
    let runtime = TestRuntime::default();

    let tmpdir = tempfile::tempdir().unwrap();

    let storage: ProverStorage<sov_state::DefaultStorageSpec> =
        ProverStorage::with_path(tmpdir.path()).unwrap();

    let mut app_template = AppTemplate::<
        C,
        TestRuntime<C>,
        MockZkvm,
        TestValidityCond,
        TestBlob<<DefaultContext as Spec>::Address>,
    >::new(storage, runtime);

    let genesis_hash_prefix = app_template
        .runtime
        .chain_state
        .genesis_hash
        .prefix()
        .clone();

    let slot_height_prefix = app_template
        .runtime
        .chain_state
        .slot_height
        .prefix()
        .clone();

    let transition_in_progress = app_template
        .runtime
        .chain_state
        .in_progress_transition
        .prefix()
        .clone();

    let value_setter_messages = ValueSetterMessages::default();
    let value_setter = value_setter_messages.create_raw_txs::<TestRuntime<C>>();

    let admin_pub_key = value_setter_messages.messages[0].admin.default_address();

    // Genesis
    let init_root_hash = app_template
        .init_chain(create_demo_genesis_config(admin_pub_key))
        .unwrap();

    const MOCK_SEQUENCER_DA_ADDRESS: [u8; 32] = [1_u8; 32];

    let blob = new_test_blob_from_batch(
        sov_modules_stf_template::Batch { txs: value_setter },
        &MOCK_SEQUENCER_DA_ADDRESS,
        [2; 32],
    );

    let slot_data: TestBlock = TestBlock {
        curr_hash: [10; 32],
        header: TestBlockHeader {
            prev_hash: TestHash([0; 32]),
        },
        height: 0,
        validity_cond: TestValidityCond::default(),
    };

    // Check the slot height before apply slot
    let new_height_storage: u64 = app_template
        .get_from_storage_with_prefix(&slot_height_prefix, &SingletonKey)
        .unwrap();

    assert_eq!(new_height_storage, 0, "The initial height was not computed");

    let result = app_template.apply_slot(Default::default(), &slot_data, &mut [blob]);

    assert_eq!(1, result.batch_receipts.len());
    let apply_blob_outcome = result.batch_receipts[0].clone();
    assert_eq!(
        SequencerOutcome::Rewarded(0),
        apply_blob_outcome.inner,
        "Sequencer execution should have succeeded but failed "
    );

    // Check that the root hash has been stored correctly
    let stored_root: [u8; 32] = app_template
        .get_from_storage_with_prefix(&genesis_hash_prefix, &SingletonKey)
        .unwrap();

    assert_eq!(stored_root, init_root_hash, "Root hashes don't match");

    // Check the slot height
    let new_height_storage: u64 = app_template
        .get_from_storage_with_prefix(&slot_height_prefix, &SingletonKey)
        .unwrap();

    assert_eq!(new_height_storage, 1, "The new height did not update");

    // Check the tx in progress
    let new_tx_in_progress: TransitionInProgress<TestValidityCond> = app_template
        .get_from_storage_with_prefix(&transition_in_progress, &SingletonKey)
        .unwrap();

    assert_eq!(
        new_tx_in_progress,
        TransitionInProgress::<TestValidityCond> {
            da_block_hash: [10; 32],
            validity_condition: TestValidityCond::default()
        },
        "The new transition has not been correctly stored"
    );

    assert!(has_tx_events(&apply_blob_outcome),);
}
