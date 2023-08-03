use sov_data_generators::value_setter_data::ValueSetterMessages;
use sov_data_generators::{has_tx_events, new_test_blob_from_batch, MessageGenerator};
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::Spec;
use sov_modules_stf_template::{AppTemplate, SequencerOutcome};
use sov_rollup_interface::mocks::{MockZkvm, TestBlob, TestBlock, TestValidityCond};
use sov_rollup_interface::stf::StateTransitionFunction;
use sov_state::ProverStorage;

use crate::tests_helpers::{create_demo_genesis_config, TestRuntime};

type C = DefaultContext;

/// This test generates a new mock rollup having a simple value setter module
/// with an associated chain state, and checks that the height, the genesis hash
/// and the state transitions are correctly stored and updated.
#[test]
fn test_value_setter() {
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

    let value_setter_messages = ValueSetterMessages::default();
    let value_setter = value_setter_messages.create_raw_txs::<TestRuntime<C>>();

    let admin_pub_key = value_setter_messages.messages[0].admin.default_address();

    // Genesis
    let _init_root_hash = app_template
        .init_chain(create_demo_genesis_config(admin_pub_key))
        .unwrap();

    const MOCK_SEQUENCER_DA_ADDRESS: [u8; 32] = [1_u8; 32];

    let blob = new_test_blob_from_batch(
        sov_modules_stf_template::Batch { txs: value_setter },
        &MOCK_SEQUENCER_DA_ADDRESS,
        [0; 32],
    );

    let slot_data: TestBlock = TestBlock::default();

    let result = app_template.apply_slot(Default::default(), &slot_data, &mut [blob]);

    assert_eq!(1, result.batch_receipts.len());
    let apply_blob_outcome = result.batch_receipts[0].clone();
    assert_eq!(
        SequencerOutcome::Rewarded(0),
        apply_blob_outcome.inner,
        "Sequencer execution should have succeeded but failed "
    );

    assert!(has_tx_events(&apply_blob_outcome),);
}
