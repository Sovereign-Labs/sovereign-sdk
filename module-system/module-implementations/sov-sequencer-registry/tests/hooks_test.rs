use sov_modules_api::hooks::ApplyBlobHooks;
use sov_state::{ProverStorage, WorkingSet};

mod helpers;

use helpers::*;
use sov_modules_api::Address;
use sov_rollup_interface::mocks::TestBlob;
use sov_sequencer_registry::SequencerOutcome;

#[test]
fn begin_blob_hook_known_sequencer() {
    let mut test_sequencer = create_test_sequencer();
    let tmpdir = tempfile::tempdir().unwrap();
    let working_set = &mut WorkingSet::new(ProverStorage::with_path(tmpdir.path()).unwrap());
    test_sequencer.genesis(working_set);

    let balance_after_genesis = {
        let resp = test_sequencer.query_balance_via_bank(working_set);
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

    let resp = test_sequencer.query_balance_via_bank(working_set);
    assert_eq!(balance_after_genesis, resp.amount.unwrap());
    let resp = test_sequencer
        .registry
        .sequencer_address(GENESIS_SEQUENCER_DA_ADDRESS.to_vec(), working_set);
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
    let expected_message_part = "Value not found for prefix: \"sov_sequencer_registry/SequencerRegistry/allowed_sequencers/\"";
    let actual_message = result.err().unwrap().to_string();
    assert!(actual_message.contains(expected_message_part));
}

#[test]
fn end_blob_hook_success() {
    let mut test_sequencer = create_test_sequencer();
    let tmpdir = tempfile::tempdir().unwrap();
    let working_set = &mut WorkingSet::new(ProverStorage::with_path(tmpdir.path()).unwrap());
    test_sequencer.genesis(working_set);
    let balance_after_genesis = {
        let resp = test_sequencer.query_balance_via_bank(working_set);
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

    test_sequencer
        .registry
        .end_blob_hook(SequencerOutcome::Completed, working_set)
        .unwrap();
    let resp = test_sequencer.query_balance_via_bank(working_set);
    assert_eq!(balance_after_genesis, resp.amount.unwrap());
    let resp = test_sequencer
        .registry
        .sequencer_address(GENESIS_SEQUENCER_DA_ADDRESS.to_vec(), working_set);
    assert!(resp.address.is_some());
}

#[test]
fn end_blob_hook_slash() {
    let mut test_sequencer = create_test_sequencer();
    let tmpdir = tempfile::tempdir().unwrap();
    let working_set = &mut WorkingSet::new(ProverStorage::with_path(tmpdir.path()).unwrap());
    test_sequencer.genesis(working_set);
    let balance_after_genesis = {
        let resp = test_sequencer.query_balance_via_bank(working_set);
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
    test_sequencer
        .registry
        .end_blob_hook(result, working_set)
        .unwrap();

    let resp = test_sequencer.query_balance_via_bank(working_set);
    assert_eq!(balance_after_genesis, resp.amount.unwrap());
    let resp = test_sequencer
        .registry
        .sequencer_address(GENESIS_SEQUENCER_DA_ADDRESS.to_vec(), working_set);
    assert!(resp.address.is_none());
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
        .sequencer_address(UNKNOWN_SEQUENCER_DA_ADDRESS.to_vec(), working_set);
    assert!(resp.address.is_none());

    let result = SequencerOutcome::Slashed {
        sequencer: UNKNOWN_SEQUENCER_DA_ADDRESS.to_vec(),
    };
    test_sequencer
        .registry
        .end_blob_hook(result, working_set)
        .unwrap();

    let resp = test_sequencer
        .registry
        .sequencer_address(UNKNOWN_SEQUENCER_DA_ADDRESS.to_vec(), working_set);
    assert!(resp.address.is_none());
}
