use crate::{
    data_generation::{simulate_da_with_bad_sig, simulate_da_with_revert_msg, QueryGenerator},
    helpers::{check_query, query_and_deserialize},
    runtime::Runtime,
    test_utils::{create_new_demo, LOCKED_AMOUNT, SEQUENCER_DA_ADDRESS},
};
use sov_app_template::Batch;
use sov_modules_api::mocks::MockContext;
use sov_state::ProverStorage;
use sovereign_sdk::stf::StateTransitionFunction;

#[test]
fn test_tx_revert() {
    let path = schemadb::temppath::TempPath::new();
    {
        let mut demo = create_new_demo(LOCKED_AMOUNT + 1, &path);

        demo.init_chain(());
        demo.begin_slot();

        let txs = simulate_da_with_revert_msg();

        demo.apply_batch(Batch { txs }, &SEQUENCER_DA_ADDRESS, None)
            .expect("Batch is valid");

        demo.end_slot();
    }

    // Checks
    {
        let runtime = &mut Runtime::<MockContext>::new();
        let storage = ProverStorage::with_path(&path).unwrap();

        // We sent 4 vote messages but one of them is invalid and should be reverted.
        check_query(
            runtime,
            QueryGenerator::generate_query_election_nb_of_votes_message(),
            r#"{"Result":3}"#,
            storage.clone(),
        );

        check_query(
            runtime,
            QueryGenerator::generate_query_election_message(),
            r#"{"Result":{"name":"candidate_2","count":3}}"#,
            storage,
        );
    }
}

#[test]
fn test_tx_bad_sig() {
    let path = schemadb::temppath::TempPath::new();

    {
        let mut demo = create_new_demo(LOCKED_AMOUNT + 1, &path);

        demo.init_chain(());
        demo.begin_slot();

        let txs = simulate_da_with_bad_sig();

        let res = demo
            .apply_batch(Batch { txs }, &SEQUENCER_DA_ADDRESS, None)
            .unwrap_err();

        assert_eq!(res.to_string(), "Stateless verification error - the sequencer included a transaction which was known to be invalid. Bad signature");

        demo.end_slot();
    }

    {
        let runtime = &mut Runtime::<MockContext>::new();
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

        assert_eq!(resp.data.unwrap().balance, 1);
    }
}
