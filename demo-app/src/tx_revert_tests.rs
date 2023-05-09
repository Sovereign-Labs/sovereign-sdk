use core::panic;

use crate::{
    app::{create_config, create_new_demo, LOCKED_AMOUNT, SEQUENCER_DA_ADDRESS},
    data_generation::{
        simulate_da_with_bad_serialization, simulate_da_with_bad_sig, simulate_da_with_revert_msg,
        QueryGenerator,
    },
    helpers::query_and_deserialize,
    runtime::Runtime,
    tests::test::TestBlob,
};
use sov_app_template::{Batch, SlashingReason};
use sov_modules_api::default_context::DefaultContext;
use sov_state::ProverStorage;
use sovereign_sdk::stf::StateTransitionFunction;

const SEQUENCER_BALANCE_DELTA: u64 = 1;
const SEQUENCER_BALANCE: u64 = LOCKED_AMOUNT + SEQUENCER_BALANCE_DELTA;

#[test]
fn test_tx_revert() {
    let path = schemadb::temppath::TempPath::new();
    {
        let mut demo = create_new_demo(&path);

        demo.init_chain(create_config(SEQUENCER_BALANCE));
        demo.begin_slot(Default::default());

        let txs = simulate_da_with_revert_msg();

        match demo
            .apply_blob(TestBlob::new(Batch { txs }, &SEQUENCER_DA_ADDRESS), None)
            .inner
        {
            sov_app_template::SequencerOutcome::Rewarded => {}
            _ => panic!("Unexpected outcome: Batch exeuction should have succeeded"),
        }

        demo.end_slot();
    }

    // Checks
    {
        let runtime = &mut Runtime::<DefaultContext>::new();
        let storage = ProverStorage::with_path(&path).unwrap();

        // We sent 4 vote messages but one of them is invalid and should be reverted.
        let resp = query_and_deserialize::<election::query::GetNbOfVotesResponse>(
            runtime,
            QueryGenerator::generate_query_election_nb_of_votes_message(),
            storage.clone(),
        );

        assert_eq!(resp, election::query::GetNbOfVotesResponse::Result(3));

        let resp = query_and_deserialize::<election::query::GetResultResponse>(
            runtime,
            QueryGenerator::generate_query_election_message(),
            storage.clone(),
        );

        assert_eq!(
            resp,
            election::query::GetResultResponse::Result(Some(election::Candidate {
                name: "candidate_2".to_owned(),
                count: 3
            }))
        );

        let resp = query_and_deserialize::<sequencer::query::SequencerAndBalanceResponse>(
            runtime,
            QueryGenerator::generate_query_check_balance(),
            storage,
        );

        // Sequencer is rewarded
        assert_eq!(resp.data.unwrap().balance, SEQUENCER_BALANCE);
    }
}

#[test]
fn test_tx_bad_sig() {
    let path = schemadb::temppath::TempPath::new();

    {
        let mut demo = create_new_demo(&path);

        demo.init_chain(create_config(SEQUENCER_BALANCE));
        demo.begin_slot(Default::default());

        let txs = simulate_da_with_bad_sig();

        match demo
            .apply_blob(TestBlob::new(Batch { txs }, &SEQUENCER_DA_ADDRESS), None).inner {
                sov_app_template::SequencerOutcome::Slashed(SlashingReason::StatelessVerificationFailed) => {}
                _ => panic!("Unexpected outcome: Stateless verification should have failed due to invalid signature")
            }

        demo.end_slot();
    }

    {
        let runtime = &mut Runtime::<DefaultContext>::new();
        let storage = ProverStorage::with_path(&path).unwrap();

        let resp = query_and_deserialize::<election::query::GetResultResponse>(
            runtime,
            QueryGenerator::generate_query_election_message(),
            storage.clone(),
        );

        assert_eq!(
            resp,
            election::query::GetResultResponse::Err("Election is not frozen".to_owned())
        );

        let resp = query_and_deserialize::<sequencer::query::SequencerAndBalanceResponse>(
            runtime,
            QueryGenerator::generate_query_check_balance(),
            storage,
        );

        // Sequencer is slashed
        assert_eq!(resp.data.unwrap().balance, SEQUENCER_BALANCE_DELTA);
    }
}

// This test is outdated, since we no longer revert on stateful verification errors
// TODO: re-enable this test with with a granular check for failure of a single transaction
// #[test]
// fn test_tx_bad_nonce() {
//     let path = schemadb::temppath::TempPath::new();

//     {
//         let mut demo = create_new_demo(&path);

//         demo.init_chain(create_config(SEQUENCER_BALANCE));
//         demo.begin_slot();

//         let txs = simulate_da_with_bad_nonce();

//         let res = demo
//             .apply_blob(TestBlob::new(Batch { txs }, &SEQUENCER_DA_ADDRESS), None)
//             .unwrap_err();

//         assert_eq!(res.to_string(), "Stateful verification error - the sequencer included an invalid transaction: Tx bad nonce, expected: 4, but found: 5");

//         demo.end_slot();
//     }

//     {
//         let runtime = &mut Runtime::<DefaultContext>::new();
//         let storage = ProverStorage::with_path(&path).unwrap();

//         let resp = query_and_deserialize::<election::query::GetResultResponse>(
//             runtime,
//             QueryGenerator::generate_query_election_message(),
//             storage.clone(),
//         );

//         assert_eq!(
//             resp,
//             election::query::GetResultResponse::Err("Election is not frozen".to_owned())
//         );

//         let resp = query_and_deserialize::<sequencer::query::SequencerAndBalanceResponse>(
//             runtime,
//             QueryGenerator::generate_query_check_balance(),
//             storage,
//         );

//         // Sequencer is rewarded
//         assert_eq!(resp.data.unwrap().balance, SEQUENCER_BALANCE);
//     }
// }

#[test]
fn test_tx_bad_serialization() {
    let path = schemadb::temppath::TempPath::new();

    {
        let mut demo = create_new_demo(&path);

        demo.init_chain(create_config(SEQUENCER_BALANCE));
        demo.begin_slot(Default::default());

        let txs = simulate_da_with_bad_serialization();

        match demo
            .apply_blob(TestBlob::new(Batch { txs }, &SEQUENCER_DA_ADDRESS), None).inner {
                sov_app_template::SequencerOutcome::Slashed(SlashingReason::InvalidTransactionEncoding) => {}
                _ => panic!("Unexpected outcome: Stateless verification should have failed due to invalid signature")
            }

        demo.end_slot();
    }

    {
        let runtime = &mut Runtime::<DefaultContext>::new();
        let storage = ProverStorage::with_path(&path).unwrap();

        let resp = query_and_deserialize::<election::query::GetResultResponse>(
            runtime,
            QueryGenerator::generate_query_election_message(),
            storage.clone(),
        );

        assert_eq!(
            resp,
            election::query::GetResultResponse::Err("Election is not frozen".to_owned())
        );

        let resp = query_and_deserialize::<sequencer::query::SequencerAndBalanceResponse>(
            runtime,
            QueryGenerator::generate_query_check_balance(),
            storage,
        );

        // Sequencer is slashed
        assert_eq!(resp.data.unwrap().balance, SEQUENCER_BALANCE_DELTA);
    }
}
