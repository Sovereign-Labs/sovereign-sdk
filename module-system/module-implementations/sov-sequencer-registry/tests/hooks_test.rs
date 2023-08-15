use sov_modules_api::hooks::ApplyBlobHooks;
use sov_state::{ProverStorage, WorkingSet};

mod helpers;

use helpers::*;
use sov_modules_api::Address;
use sov_rollup_interface::mocks::TestBlob;
use sov_sequencer_registry::{SequencerOutcome, SequencerRegistry};

#[test]
fn begin_blob_hook_known_sequencer() {
    let mut test_sequencer = create_test_sequencer();
    let tmpdir = tempfile::tempdir().unwrap();
    let working_set = &mut WorkingSet::new(ProverStorage::with_path(tmpdir.path()).unwrap());
    test_sequencer.genesis(working_set);

    let balance_after_genesis = {
        let resp = test_sequencer.query_balance_via_bank(working_set).unwrap();
        resp.amount.unwrap()
    };
    assert_eq!(INITIAL_BALANCE - LOCKED_AMOUNT, balance_after_genesis);

    let mut test_blob = TestBlob::new(
        Vec::new(),
        Address::from(GENESIS_SEQUENCER_DA_ADDRESS),
        [0_u8; 32],
    );

    test_sequencer
        .registry
        .begin_blob_hook(&mut test_blob, working_set)
        .unwrap();

    let resp = test_sequencer.query_balance_via_bank(working_set).unwrap();
    assert_eq!(balance_after_genesis, resp.amount.unwrap());
    let resp = test_sequencer
        .registry
        .sequencer_address(GENESIS_SEQUENCER_DA_ADDRESS.to_vec(), working_set)
        .unwrap();
    assert!(resp.address.is_some());
}

#[test]
fn begin_blob_hook_unknown_sequencer() {
    let mut test_sequencer = create_test_sequencer();
    let tmpdir = tempfile::tempdir().unwrap();
    let working_set = &mut WorkingSet::new(ProverStorage::with_path(tmpdir.path()).unwrap());
    test_sequencer.genesis(working_set);

    let mut test_blob = TestBlob::new(
        Vec::new(),
        Address::from(UNKNOWN_SEQUENCER_DA_ADDRESS),
        [0_u8; 32],
    );

    let result = test_sequencer
        .registry
        .begin_blob_hook(&mut test_blob, working_set);
    assert!(result.is_err());
    let expected_message = format!(
        "sender {} is not allowed to submit blobs",
        Address::from(UNKNOWN_SEQUENCER_DA_ADDRESS)
    );
    let actual_message = result.err().unwrap().to_string();
    assert_eq!(expected_message, actual_message);
}

#[test]
fn end_blob_hook_success() {
    let mut test_sequencer = create_test_sequencer();
    let tmpdir = tempfile::tempdir().unwrap();
    let working_set = &mut WorkingSet::new(ProverStorage::with_path(tmpdir.path()).unwrap());
    test_sequencer.genesis(working_set);
    let balance_after_genesis = {
        let resp = test_sequencer.query_balance_via_bank(working_set).unwrap();
        resp.amount.unwrap()
    };
    assert_eq!(INITIAL_BALANCE - LOCKED_AMOUNT, balance_after_genesis);

    let mut test_blob = TestBlob::new(
        Vec::new(),
        Address::from(GENESIS_SEQUENCER_DA_ADDRESS),
        [0_u8; 32],
    );

    test_sequencer
        .registry
        .begin_blob_hook(&mut test_blob, working_set)
        .unwrap();

    <SequencerRegistry<C> as ApplyBlobHooks<TestBlob<Address>>>::end_blob_hook(
        &test_sequencer.registry,
        SequencerOutcome::Completed,
        working_set,
    )
    .unwrap();
    let resp = test_sequencer.query_balance_via_bank(working_set).unwrap();
    assert_eq!(balance_after_genesis, resp.amount.unwrap());
    let resp = test_sequencer
        .registry
        .sequencer_address(GENESIS_SEQUENCER_DA_ADDRESS.to_vec(), working_set)
        .unwrap();
    assert!(resp.address.is_some());
}

#[test]
fn end_blob_hook_slash() {
    let mut test_sequencer = create_test_sequencer();
    let tmpdir = tempfile::tempdir().unwrap();
    let working_set = &mut WorkingSet::new(ProverStorage::with_path(tmpdir.path()).unwrap());
    test_sequencer.genesis(working_set);
    let balance_after_genesis = {
        let resp = test_sequencer.query_balance_via_bank(working_set).unwrap();
        resp.amount.unwrap()
    };
    assert_eq!(INITIAL_BALANCE - LOCKED_AMOUNT, balance_after_genesis);

    let mut test_blob = TestBlob::new(
        Vec::new(),
        Address::from(GENESIS_SEQUENCER_DA_ADDRESS),
        [0_u8; 32],
    );

    test_sequencer
        .registry
        .begin_blob_hook(&mut test_blob, working_set)
        .unwrap();

    let result = SequencerOutcome::Slashed {
        sequencer: GENESIS_SEQUENCER_DA_ADDRESS.to_vec(),
    };
    <SequencerRegistry<C> as ApplyBlobHooks<TestBlob<Address>>>::end_blob_hook(
        &test_sequencer.registry,
        result,
        working_set,
    )
    .unwrap();

    let resp = test_sequencer.query_balance_via_bank(working_set).unwrap();
    assert_eq!(balance_after_genesis, resp.amount.unwrap());
    let resp = test_sequencer
        .registry
        .sequencer_address(GENESIS_SEQUENCER_DA_ADDRESS.to_vec(), working_set)
        .unwrap();
    assert!(resp.address.is_none());
}

#[test]
fn end_blob_hook_slash_preferred_sequencer() {
    let bank = sov_bank::Bank::<C>::default();
    let (bank_config, seq_rollup_address) = create_bank_config();

    let token_address = sov_bank::get_genesis_token_address::<C>(
        &bank_config.tokens[0].token_name,
        bank_config.tokens[0].salt,
    );

    let registry = SequencerRegistry::<C>::default();
    let mut sequencer_config = create_sequencer_config(seq_rollup_address, token_address);

    sequencer_config.preferred_sequencer = Some(sequencer_config.seq_da_address.clone());

    let mut test_sequencer = TestSequencer {
        bank,
        bank_config,
        registry,
        sequencer_config,
    };

    let tmpdir = tempfile::tempdir().unwrap();
    let working_set = &mut WorkingSet::new(ProverStorage::with_path(tmpdir.path()).unwrap());
    test_sequencer.genesis(working_set);
    let balance_after_genesis = {
        let resp = test_sequencer.query_balance_via_bank(working_set).unwrap();
        resp.amount.unwrap()
    };
    assert_eq!(INITIAL_BALANCE - LOCKED_AMOUNT, balance_after_genesis);

    let mut test_blob = TestBlob::new(
        Vec::new(),
        Address::from(GENESIS_SEQUENCER_DA_ADDRESS),
        [0_u8; 32],
    );

    test_sequencer
        .registry
        .begin_blob_hook(&mut test_blob, working_set)
        .unwrap();

    let result = SequencerOutcome::Slashed {
        sequencer: GENESIS_SEQUENCER_DA_ADDRESS.to_vec(),
    };
    <SequencerRegistry<C> as ApplyBlobHooks<TestBlob<Address>>>::end_blob_hook(
        &test_sequencer.registry,
        result,
        working_set,
    )
    .unwrap();

    let resp = test_sequencer.query_balance_via_bank(working_set).unwrap();
    assert_eq!(balance_after_genesis, resp.amount.unwrap());
    let resp = test_sequencer
        .registry
        .sequencer_address(GENESIS_SEQUENCER_DA_ADDRESS.to_vec(), working_set)
        .unwrap();
    assert!(resp.address.is_none());

    assert!(test_sequencer
        .registry
        .get_preferred_sequencer(working_set)
        .is_none());
}

#[test]
fn end_blob_hook_slash_unknown_sequencer() {
    let mut test_sequencer = create_test_sequencer();
    let tmpdir = tempfile::tempdir().unwrap();
    let working_set = &mut WorkingSet::new(ProverStorage::with_path(tmpdir.path()).unwrap());
    test_sequencer.genesis(working_set);

    let mut test_blob = TestBlob::new(
        Vec::new(),
        Address::from(GENESIS_SEQUENCER_DA_ADDRESS),
        [0_u8; 32],
    );

    test_sequencer
        .registry
        .begin_blob_hook(&mut test_blob, working_set)
        .unwrap();

    let resp = test_sequencer
        .registry
        .sequencer_address(UNKNOWN_SEQUENCER_DA_ADDRESS.to_vec(), working_set)
        .unwrap();
    assert!(resp.address.is_none());

    let result = SequencerOutcome::Slashed {
        sequencer: UNKNOWN_SEQUENCER_DA_ADDRESS.to_vec(),
    };
    <SequencerRegistry<C> as ApplyBlobHooks<TestBlob<Address>>>::end_blob_hook(
        &test_sequencer.registry,
        result,
        working_set,
    )
    .unwrap();

    let resp = test_sequencer
        .registry
        .sequencer_address(UNKNOWN_SEQUENCER_DA_ADDRESS.to_vec(), working_set)
        .unwrap();
    assert!(resp.address.is_none());
}
