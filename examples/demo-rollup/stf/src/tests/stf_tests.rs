use sov_cli::wallet_state::PrivateKeyAndAddress;
use sov_data_generators::bank_data::get_default_token_address;
use sov_data_generators::{has_tx_events, new_test_blob_from_batch};
use sov_mock_da::{MockBlock, MockDaSpec, MOCK_SEQUENCER_DA_ADDRESS};
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::default_signature::private_key::DefaultPrivateKey;
use sov_modules_api::{Context, PrivateKey, WorkingSet};
use sov_modules_stf_blueprint::{Batch, SequencerOutcome, StfBlueprint};
use sov_rollup_interface::services::da::SlotData;
use sov_rollup_interface::stf::StateTransitionFunction;
use sov_rollup_interface::storage::HierarchicalStorageManager;

use crate::runtime::Runtime;
use crate::tests::da_simulation::simulate_da;
use crate::tests::{
    create_storage_manager_for_tests, get_genesis_config_for_tests, StfBlueprintTest, C,
};

#[test]
fn test_demo_values_in_db() {
    let tempdir = tempfile::tempdir().unwrap();
    let path = tempdir.path();
    let mut storage_manager = create_storage_manager_for_tests(path);
    let config = get_genesis_config_for_tests();

    let genesis_block = MockBlock::default();
    let block_1 = genesis_block.next_mock();

    let last_block = {
        let stf: StfBlueprintTest = StfBlueprint::new();
        let storage = storage_manager
            .create_storage_on(genesis_block.header())
            .unwrap();
        let (genesis_root, storage) = stf.init_chain(storage, config);
        storage_manager
            .save_change_set(genesis_block.header(), storage)
            .unwrap();

        let priv_key = read_private_key::<DefaultContext>().private_key;
        let txs = simulate_da(priv_key);
        let blob = new_test_blob_from_batch(Batch { txs }, &MOCK_SEQUENCER_DA_ADDRESS, [0; 32]);

        let mut blobs = [blob];

        let storage = storage_manager.create_storage_on(block_1.header()).unwrap();

        let result = stf.apply_slot(
            &genesis_root,
            storage,
            Default::default(),
            &block_1.header,
            &block_1.validity_cond,
            &mut blobs,
        );
        assert_eq!(1, result.batch_receipts.len());
        // 2 transactions from value setter
        // 2 transactions from bank
        assert_eq!(4, result.batch_receipts[0].tx_receipts.len());

        let apply_blob_outcome = result.batch_receipts[0].clone();
        assert_eq!(
            SequencerOutcome::Rewarded(0),
            apply_blob_outcome.inner,
            "Sequencer execution should have succeeded but failed "
        );

        assert!(has_tx_events(&apply_blob_outcome),);
        storage_manager
            .save_change_set(block_1.header(), result.change_set)
            .unwrap();
        block_1
    };

    // Generate a new storage instance after dumping data to the db.
    {
        let next_block = last_block.next_mock();
        let runtime = &mut Runtime::<DefaultContext, MockDaSpec>::default();
        let storage = storage_manager
            .create_storage_on(next_block.header())
            .unwrap();
        let mut working_set = WorkingSet::new(storage);
        let resp = runtime
            .bank
            .supply_of(None, get_default_token_address(), &mut working_set)
            .unwrap();
        assert_eq!(resp, sov_bank::TotalSupplyResponse { amount: Some(1000) });

        let resp = runtime.value_setter.query_value(&mut working_set).unwrap();

        assert_eq!(resp, sov_value_setter::Response { value: Some(33) });
    }
}

#[test]
fn test_demo_values_in_cache() {
    let tempdir = tempfile::tempdir().unwrap();
    let path = tempdir.path();
    let mut storage_manager = create_storage_manager_for_tests(path);

    let stf: StfBlueprintTest = StfBlueprint::new();

    let config = get_genesis_config_for_tests();

    let genesis_block = MockBlock::default();
    let storage = storage_manager
        .create_storage_on(genesis_block.header())
        .unwrap();
    let (genesis_root, storage) = stf.init_chain(storage, config);
    storage_manager
        .save_change_set(genesis_block.header(), storage)
        .unwrap();

    let private_key = read_private_key::<DefaultContext>().private_key;
    let txs = simulate_da(private_key);

    let blob = new_test_blob_from_batch(Batch { txs }, &MOCK_SEQUENCER_DA_ADDRESS, [0; 32]);
    let mut blobs = [blob];
    let block_1 = genesis_block.next_mock();
    let storage = storage_manager.create_storage_on(block_1.header()).unwrap();

    let apply_block_result = stf.apply_slot(
        &genesis_root,
        storage,
        Default::default(),
        &block_1.header,
        &block_1.validity_cond,
        &mut blobs,
    );

    assert_eq!(1, apply_block_result.batch_receipts.len());
    let apply_blob_outcome = apply_block_result.batch_receipts[0].clone();

    assert_eq!(
        SequencerOutcome::Rewarded(0),
        apply_blob_outcome.inner,
        "Sequencer execution should have succeeded but failed"
    );

    assert!(has_tx_events(&apply_blob_outcome),);

    let runtime = &mut Runtime::<DefaultContext, MockDaSpec>::default();
    let mut working_set = WorkingSet::new(apply_block_result.change_set);

    let resp = runtime
        .bank
        .supply_of(None, get_default_token_address(), &mut working_set)
        .unwrap();
    assert_eq!(resp, sov_bank::TotalSupplyResponse { amount: Some(1000) });

    let resp = runtime.value_setter.query_value(&mut working_set).unwrap();

    assert_eq!(resp, sov_value_setter::Response { value: Some(33) });
}

#[test]
#[ignore = "end_slot is removed from STF trait"]
fn test_demo_values_not_in_db() {
    let tempdir = tempfile::tempdir().unwrap();
    let path = tempdir.path();
    let mut storage_manager = create_storage_manager_for_tests(path);

    let value_setter_admin_private_key = DefaultPrivateKey::generate();
    let genesis_block = MockBlock::default();
    let block_1 = genesis_block.next_mock();
    let block_2 = block_1.next_mock();

    let config = get_genesis_config_for_tests();
    {
        let stf: StfBlueprintTest = StfBlueprint::new();

        let storage = storage_manager
            .create_storage_on(genesis_block.header())
            .unwrap();
        let (genesis_root, storage) = stf.init_chain(storage, config);
        storage_manager
            .save_change_set(genesis_block.header(), storage)
            .unwrap();

        let txs = simulate_da(value_setter_admin_private_key);
        let blob = new_test_blob_from_batch(Batch { txs }, &MOCK_SEQUENCER_DA_ADDRESS, [0; 32]);
        let mut blobs = [blob];

        let storage = storage_manager.create_storage_on(block_1.header()).unwrap();

        let apply_block_result = stf.apply_slot(
            &genesis_root,
            storage,
            Default::default(),
            &block_1.header,
            &block_1.validity_cond,
            &mut blobs,
        );

        assert_eq!(1, apply_block_result.batch_receipts.len());
        let apply_blob_outcome = apply_block_result.batch_receipts[0].clone();

        assert_eq!(
            SequencerOutcome::Rewarded(0),
            apply_blob_outcome.inner,
            "Sequencer execution should have succeeded but failed",
        );
    }

    // Generate a new storage instance,
    // values are missing because change set from apply slot wasn't saved back to storage manager
    {
        let runtime = &mut Runtime::<C, MockDaSpec>::default();
        let storage = storage_manager.create_storage_on(block_2.header()).unwrap();
        let mut working_set = WorkingSet::new(storage);

        let resp = runtime
            .bank
            .supply_of(None, get_default_token_address(), &mut working_set)
            .unwrap();
        assert_eq!(resp, sov_bank::TotalSupplyResponse { amount: Some(1000) });

        let resp = runtime.value_setter.query_value(&mut working_set).unwrap();

        assert_eq!(resp, sov_value_setter::Response { value: None });
    }
}

#[test]
fn test_sequencer_unknown_sequencer() {
    let tempdir = tempfile::tempdir().unwrap();
    let path = tempdir.path();

    let mut config = get_genesis_config_for_tests();
    config.runtime.sequencer_registry.is_preferred_sequencer = false;

    let genesis_block = MockBlock::default();
    let block_1 = genesis_block.next_mock();

    let mut storage_manager = create_storage_manager_for_tests(path);
    let stf: StfBlueprintTest = StfBlueprint::new();
    let (genesis_root, storage) = stf.init_chain(
        storage_manager
            .create_storage_on(genesis_block.header())
            .unwrap(),
        config,
    );
    storage_manager
        .save_change_set(genesis_block.header(), storage)
        .unwrap();

    let some_sequencer: [u8; 32] = [121; 32];

    let private_key = read_private_key::<DefaultContext>().private_key;
    let txs = simulate_da(private_key);
    let blob = new_test_blob_from_batch(Batch { txs }, &some_sequencer, [0; 32]);
    let mut blobs = [blob];

    let storage = storage_manager.create_storage_on(block_1.header()).unwrap();

    let apply_block_result = stf.apply_slot(
        &genesis_root,
        storage,
        Default::default(),
        &block_1.header,
        &block_1.validity_cond,
        &mut blobs,
    );

    assert_eq!(1, apply_block_result.batch_receipts.len());
    let apply_blob_outcome = apply_block_result.batch_receipts[0].clone();

    assert_eq!(
        SequencerOutcome::Ignored,
        apply_blob_outcome.inner,
        "Batch should have been skipped due to unknown sequencer"
    );

    // Assert that there are no events
    assert!(!has_tx_events(&apply_blob_outcome));
}

fn read_private_key<C: Context>() -> PrivateKeyAndAddress<C> {
    let token_deployer_data =
        std::fs::read_to_string("../../test-data/keys/token_deployer_private_key.json")
            .expect("Unable to read file to string");

    let token_deployer: PrivateKeyAndAddress<C> = serde_json::from_str(&token_deployer_data)
        .unwrap_or_else(|_| {
            panic!(
                "Unable to convert data {} to PrivateKeyAndAddress",
                &token_deployer_data
            )
        });

    assert!(
        token_deployer.is_matching_to_default(),
        "Inconsistent key data"
    );

    token_deployer
}
