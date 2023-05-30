use crate::{
    genesis_config::{DEMO_SEQUENCER_DA_ADDRESS, LOCKED_AMOUNT},
    runtime::Runtime,
    tests::{data_generation::simulate_da_with_bad_serialization, has_tx_events},
};
use sov_default_stf::{Batch, SequencerOutcome, SlashingReason};
use sov_modules_api::{
    default_context::DefaultContext, default_signature::private_key::DefaultPrivateKey,
};
use sov_rollup_interface::{mocks::MockZkvm, stf::StateTransitionFunction};
use sov_state::{ProverStorage, WorkingSet};

use super::{
    create_demo_config, create_new_demo,
    data_generation::{simulate_da_with_bad_sig, simulate_da_with_revert_msg},
    new_test_blob,
};

const SEQUENCER_BALANCE_DELTA: u64 = 1;
const SEQUENCER_BALANCE: u64 = LOCKED_AMOUNT + SEQUENCER_BALANCE_DELTA;

#[test]
fn test_tx_revert() {
    let path = sov_schema_db::temppath::TempPath::new();
    let value_setter_admin_private_key = DefaultPrivateKey::generate();
    let election_admin_private_key = DefaultPrivateKey::generate();

    let config = create_demo_config(
        SEQUENCER_BALANCE,
        &value_setter_admin_private_key,
        &election_admin_private_key,
    );

    {
        let mut demo = create_new_demo(&path);

        StateTransitionFunction::<MockZkvm>::init_chain(&mut demo, config);
        StateTransitionFunction::<MockZkvm>::begin_slot(&mut demo, Default::default());

        let txs = simulate_da_with_revert_msg(election_admin_private_key);

        let apply_blob_outcome = StateTransitionFunction::<MockZkvm>::apply_blob(
            &mut demo,
            new_test_blob(Batch { txs }, &DEMO_SEQUENCER_DA_ADDRESS),
            None,
        );

        assert!(
            matches!(apply_blob_outcome.inner, SequencerOutcome::Rewarded(0),),
            "Unexpected outcome: Batch exeuction should have succeeded"
        );

        // Some events were observed
        assert!(has_tx_events(&apply_blob_outcome));

        StateTransitionFunction::<MockZkvm>::end_slot(&mut demo);
    }

    // Checks
    {
        let runtime = &mut Runtime::<DefaultContext>::new();
        let storage = ProverStorage::with_path(&path).unwrap();
        let mut working_set = WorkingSet::new(storage);

        // We sent 4 vote messages but one of them is invalid and should be reverted.
        let resp = runtime.election.number_of_votes(&mut working_set);

        assert_eq!(resp, sov_election::query::GetNbOfVotesResponse::Result(3));

        let resp = runtime.election.results(&mut working_set);

        assert_eq!(
            resp,
            sov_election::query::GetResultResponse::Result(Some(sov_election::Candidate {
                name: "candidate_2".to_owned(),
                count: 3
            }))
        );

        let resp = runtime
            .sequencer
            .sequencer_address_and_balance(&mut working_set);
        // Sequencer is rewarded
        assert_eq!(resp.data.unwrap().balance, SEQUENCER_BALANCE);
    }
}

#[test]
fn test_tx_bad_sig() {
    let path = sov_schema_db::temppath::TempPath::new();
    let value_setter_admin_private_key = DefaultPrivateKey::generate();
    let election_admin_private_key = DefaultPrivateKey::generate();

    let config = create_demo_config(
        SEQUENCER_BALANCE,
        &value_setter_admin_private_key,
        &election_admin_private_key,
    );

    {
        let mut demo = create_new_demo(&path);

        StateTransitionFunction::<MockZkvm>::init_chain(&mut demo, config);
        StateTransitionFunction::<MockZkvm>::begin_slot(&mut demo, Default::default());

        let txs = simulate_da_with_bad_sig(election_admin_private_key);

        let apply_blob_outcome = StateTransitionFunction::<MockZkvm>::apply_blob(
            &mut demo,
            new_test_blob(Batch { txs }, &DEMO_SEQUENCER_DA_ADDRESS),
            None,
        );

        assert!(
            matches!(apply_blob_outcome.inner, SequencerOutcome::Slashed(SlashingReason::StatelessVerificationFailed),),
            "Unexpected outcome: Stateless verification should have failed due to invalid signature"
        );

        // The batch receipt contains no events.
        assert!(!has_tx_events(&apply_blob_outcome));

        StateTransitionFunction::<MockZkvm>::end_slot(&mut demo);
    }

    {
        let runtime = &mut Runtime::<DefaultContext>::new();
        let storage = ProverStorage::with_path(&path).unwrap();
        let mut working_set = WorkingSet::new(storage);

        let resp = runtime.election.results(&mut working_set);

        assert_eq!(
            resp,
            sov_election::query::GetResultResponse::Err("Election is not frozen".to_owned())
        );

        let resp = runtime
            .sequencer
            .sequencer_address_and_balance(&mut working_set);

        // Sequencer is slashed
        assert_eq!(resp.data.unwrap().balance, SEQUENCER_BALANCE_DELTA);
    }
}

#[test]
fn test_tx_bad_serialization() {
    let path = sov_schema_db::temppath::TempPath::new();

    let value_setter_admin_private_key = DefaultPrivateKey::generate();
    let election_admin_private_key = DefaultPrivateKey::generate();

    let config = create_demo_config(
        SEQUENCER_BALANCE,
        &value_setter_admin_private_key,
        &election_admin_private_key,
    );

    {
        let mut demo = create_new_demo(&path);

        StateTransitionFunction::<MockZkvm>::init_chain(&mut demo, config);
        StateTransitionFunction::<MockZkvm>::begin_slot(&mut demo, Default::default());

        let txs = simulate_da_with_bad_serialization(election_admin_private_key);

        let apply_blob_outcome = StateTransitionFunction::<MockZkvm>::apply_blob(
            &mut demo,
            new_test_blob(Batch { txs }, &DEMO_SEQUENCER_DA_ADDRESS),
            None,
        );

        assert!(
            matches!(apply_blob_outcome.inner, sov_default_stf::SequencerOutcome::Slashed(SlashingReason::InvalidTransactionEncoding)),
            "Unexpected outcome: Stateless verification should have failed due to invalid signature"
        );

        // The batch receipt contains no events.
        assert!(!has_tx_events(&apply_blob_outcome));
        StateTransitionFunction::<MockZkvm>::end_slot(&mut demo);
    }

    {
        let runtime = &mut Runtime::<DefaultContext>::new();
        let storage = ProverStorage::with_path(&path).unwrap();
        let mut working_set = WorkingSet::new(storage);

        let resp = runtime.election.results(&mut working_set);

        assert_eq!(
            resp,
            sov_election::query::GetResultResponse::Err("Election is not frozen".to_owned())
        );

        let resp = runtime
            .sequencer
            .sequencer_address_and_balance(&mut working_set);

        // Sequencer is slashed
        assert_eq!(resp.data.unwrap().balance, SEQUENCER_BALANCE_DELTA);
    }
}
