use crate::{
    genesis_config::{DEMO_SEQUENCER_DA_ADDRESS, LOCKED_AMOUNT},
    runtime::Runtime,
    tests::{data_generation::simulate_da_with_bad_serialization, has_tx_events},
};
use borsh::BorshSerialize;
use sov_accounts::query::Response;
use sov_modules_api::transaction::Transaction;
use sov_modules_api::{
    default_context::DefaultContext, default_signature::private_key::DefaultPrivateKey, PublicKey,
};
use sov_modules_stf_template::RawTx;
use sov_modules_stf_template::{Batch, SequencerOutcome, SlashingReason};
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
    let tempdir = tempfile::tempdir().unwrap();
    let path = tempdir.path();
    let value_setter_admin_private_key = DefaultPrivateKey::generate();
    let election_admin_private_key = DefaultPrivateKey::generate();

    let config = create_demo_config(
        SEQUENCER_BALANCE,
        &value_setter_admin_private_key,
        &election_admin_private_key,
    );

    {
        let mut demo = create_new_demo(path);

        StateTransitionFunction::<MockZkvm>::init_chain(&mut demo, config);
        StateTransitionFunction::<MockZkvm>::begin_slot(&mut demo, Default::default());

        let txs = simulate_da_with_revert_msg(election_admin_private_key);

        let apply_blob_outcome = StateTransitionFunction::<MockZkvm>::apply_blob(
            &mut demo,
            &mut new_test_blob(Batch { txs }, &DEMO_SEQUENCER_DA_ADDRESS),
            None,
        );

        assert_eq!(
            SequencerOutcome::Rewarded(0),
            apply_blob_outcome.inner,
            "Unexpected outcome: Batch execution should have succeeded",
        );

        // Some events were observed
        assert!(has_tx_events(&apply_blob_outcome), "No events were taken");

        StateTransitionFunction::<MockZkvm>::end_slot(&mut demo);
    }

    // Checks
    {
        let runtime = &mut Runtime::<DefaultContext>::default();
        let storage = ProverStorage::with_path(path).unwrap();
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
fn test_nonce_incremented_on_revert() {
    let tempdir = tempfile::tempdir().unwrap();
    let path = tempdir.path();
    let value_setter_admin_private_key = DefaultPrivateKey::generate();
    let election_admin_private_key = DefaultPrivateKey::generate();
    let voter = DefaultPrivateKey::generate();

    let config = create_demo_config(
        SEQUENCER_BALANCE,
        &value_setter_admin_private_key,
        &election_admin_private_key,
    );

    {
        let mut demo = create_new_demo(path);
        StateTransitionFunction::<MockZkvm>::init_chain(&mut demo, config);
        StateTransitionFunction::<MockZkvm>::begin_slot(&mut demo, Default::default());

        let set_candidates_message = Runtime::<DefaultContext>::encode_election_call(
            sov_election::call::CallMessage::SetCandidates {
                names: vec!["candidate_1".to_owned(), "candidate_2".to_owned()],
            },
        );

        let set_candidates_message = Transaction::<DefaultContext>::new_signed_tx(
            &election_admin_private_key,
            set_candidates_message,
            0,
        );

        let add_voter_message = Runtime::<DefaultContext>::encode_election_call(
            sov_election::call::CallMessage::AddVoter(voter.pub_key().to_address()),
        );
        let add_voter_message = Transaction::<DefaultContext>::new_signed_tx(
            &election_admin_private_key,
            add_voter_message,
            1,
        );

        let vote_message = Runtime::<DefaultContext>::encode_election_call(
            sov_election::call::CallMessage::Vote(100),
        );
        let vote_message = Transaction::<DefaultContext>::new_signed_tx(&voter, vote_message, 0);

        let txs = vec![set_candidates_message, add_voter_message, vote_message];
        let txs = txs
            .into_iter()
            .map(|t| RawTx {
                data: t.try_to_vec().unwrap(),
            })
            .collect();

        let apply_blob_outcome = StateTransitionFunction::<MockZkvm>::apply_blob(
            &mut demo,
            new_test_blob(Batch { txs }, &DEMO_SEQUENCER_DA_ADDRESS),
            None,
        );

        assert_eq!(
            SequencerOutcome::Rewarded(0),
            apply_blob_outcome.inner,
            "Unexpected outcome: Batch execution should have succeeded",
        );
        StateTransitionFunction::<MockZkvm>::end_slot(&mut demo);
    }

    {
        let runtime = &mut Runtime::<DefaultContext>::default();
        let storage = ProverStorage::with_path(path).unwrap();
        let mut working_set = WorkingSet::new(storage);

        // We sent 4 vote messages but one of them is invalid and should be reverted.
        let resp = runtime.election.number_of_votes(&mut working_set);

        assert_eq!(resp, sov_election::query::GetNbOfVotesResponse::Result(0));

        let nonce = match runtime
            .accounts
            .get_account(voter.pub_key(), &mut working_set)
        {
            Response::AccountExists { nonce, .. } => nonce,
            Response::AccountEmpty => 0,
        };

        // Voter should have its nonce implemented
        assert_eq!(1, nonce);
    }
}

#[test]
fn test_tx_bad_sig() {
    let tempdir = tempfile::tempdir().unwrap();
    let path = tempdir.path();
    let value_setter_admin_private_key = DefaultPrivateKey::generate();
    let election_admin_private_key = DefaultPrivateKey::generate();

    let config = create_demo_config(
        SEQUENCER_BALANCE,
        &value_setter_admin_private_key,
        &election_admin_private_key,
    );

    {
        let mut demo = create_new_demo(path);

        StateTransitionFunction::<MockZkvm>::init_chain(&mut demo, config);
        StateTransitionFunction::<MockZkvm>::begin_slot(&mut demo, Default::default());

        let txs = simulate_da_with_bad_sig(election_admin_private_key);

        let apply_blob_outcome = StateTransitionFunction::<MockZkvm>::apply_blob(
            &mut demo,
            &mut new_test_blob(Batch { txs }, &DEMO_SEQUENCER_DA_ADDRESS),
            None,
        );

        assert_eq!(
            SequencerOutcome::Slashed(SlashingReason::StatelessVerificationFailed),
            apply_blob_outcome.inner,
            "Unexpected outcome: Stateless verification should have failed due to invalid signature"
        );

        // The batch receipt contains no events.
        assert!(!has_tx_events(&apply_blob_outcome));

        StateTransitionFunction::<MockZkvm>::end_slot(&mut demo);
    }

    {
        let runtime = &mut Runtime::<DefaultContext>::default();
        let storage = ProverStorage::with_path(path).unwrap();
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
    let tempdir = tempfile::tempdir().unwrap();
    let path = tempdir.path();

    let value_setter_admin_private_key = DefaultPrivateKey::generate();
    let election_admin_private_key = DefaultPrivateKey::generate();

    let config = create_demo_config(
        SEQUENCER_BALANCE,
        &value_setter_admin_private_key,
        &election_admin_private_key,
    );

    {
        let mut demo = create_new_demo(path);

        StateTransitionFunction::<MockZkvm>::init_chain(&mut demo, config);
        StateTransitionFunction::<MockZkvm>::begin_slot(&mut demo, Default::default());

        let txs = simulate_da_with_bad_serialization(election_admin_private_key);

        let apply_blob_outcome = StateTransitionFunction::<MockZkvm>::apply_blob(
            &mut demo,
            &mut new_test_blob(Batch { txs }, &DEMO_SEQUENCER_DA_ADDRESS),
            None,
        );

        assert_eq!(
            SequencerOutcome::Slashed(SlashingReason::InvalidTransactionEncoding),
            apply_blob_outcome.inner,
            "Unexpected outcome: Stateless verification should have failed due to invalid signature"
        );

        // The batch receipt contains no events.
        assert!(!has_tx_events(&apply_blob_outcome));
        StateTransitionFunction::<MockZkvm>::end_slot(&mut demo);
    }

    {
        let runtime = &mut Runtime::<DefaultContext>::default();
        let storage = ProverStorage::with_path(path).unwrap();
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
