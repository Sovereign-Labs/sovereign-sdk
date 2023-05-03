#[cfg(test)]
mod test {
    use sov_app_template::Batch;
    use sov_modules_api::mocks::MockContext;
    use sov_state::ProverStorage;
    use sovereign_sdk::stf::StateTransitionFunction;

    use crate::{
        app::{create_new_demo, C, LOCKED_AMOUNT, SEQUENCER_DA_ADDRESS},
        data_generation::{simulate_da, QueryGenerator},
        helpers::query_and_deserialize,
        runtime::Runtime,
    };

    #[test]
    fn test_demo_values_in_db() {
        let path = schemadb::temppath::TempPath::new();
        {
            let mut demo = create_new_demo(LOCKED_AMOUNT + 1, &path);

            demo.init_chain(());
            demo.begin_slot();

            let txs = simulate_da();

            demo.apply_batch(Batch { txs }, &SEQUENCER_DA_ADDRESS, None)
                .expect("Batch is valid");

            demo.end_slot();
        }

        // Generate a new storage instance after dumping data to the db.
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
                election::query::GetResultResponse::Result(Some(election::Candidate {
                    name: "candidate_2".to_owned(),
                    count: 3
                }))
            );

            let resp = query_and_deserialize::<value_setter::query::Response>(
                runtime,
                QueryGenerator::generate_query_value_setter_message(),
                storage,
            );

            assert_eq!(resp, value_setter::query::Response { value: Some(33) });
        }
    }

    #[test]
    fn test_demo_values_in_cache() {
        let path = schemadb::temppath::TempPath::new();
        let mut demo = create_new_demo(LOCKED_AMOUNT + 1, &path);

        demo.init_chain(());
        demo.begin_slot();

        let txs = simulate_da();

        demo.apply_batch(Batch { txs }, &SEQUENCER_DA_ADDRESS, None)
            .expect("Batch is valid");
        demo.end_slot();

        let runtime = &mut Runtime::<MockContext>::new();
        let resp = query_and_deserialize::<election::query::GetResultResponse>(
            runtime,
            QueryGenerator::generate_query_election_message(),
            demo.current_storage.clone(),
        );

        assert_eq!(
            resp,
            election::query::GetResultResponse::Result(Some(election::Candidate {
                name: "candidate_2".to_owned(),
                count: 3
            }))
        );

        let resp = query_and_deserialize::<value_setter::query::Response>(
            runtime,
            QueryGenerator::generate_query_value_setter_message(),
            demo.current_storage.clone(),
        );

        assert_eq!(resp, value_setter::query::Response { value: Some(33) });
    }

    #[test]
    fn test_demo_values_not_in_db() {
        let path = schemadb::temppath::TempPath::new();
        {
            let mut demo = create_new_demo(LOCKED_AMOUNT + 1, &path);

            demo.init_chain(());
            demo.begin_slot();

            let txs = simulate_da();

            demo.apply_batch(Batch { txs }, &SEQUENCER_DA_ADDRESS, None)
                .expect("Batch is valid");
        }

        // Generate a new storage instance, value are missing because we didn't call `end_slot()`;
        {
            let runtime = &mut Runtime::<C>::new();
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

            let resp = query_and_deserialize::<value_setter::query::Response>(
                runtime,
                QueryGenerator::generate_query_value_setter_message(),
                storage,
            );

            assert_eq!(resp, value_setter::query::Response { value: None });
        }
    }

    #[test]
    fn test_sequencer_insufficient_funds() {
        let path = schemadb::temppath::TempPath::new();
        let mut demo = create_new_demo(LOCKED_AMOUNT - 1, &path);

        demo.init_chain(());
        demo.begin_slot();

        let txs = simulate_da();

        let err = demo
            .apply_batch(Batch { txs }, &SEQUENCER_DA_ADDRESS, None)
            .unwrap_err();

        assert_eq!(
            err.to_string(),
            "Error: The transaction was rejected by the 'enter_apply_batch' hook. Insufficient funds for sov1hvyghdfvsmz4lvpd6k89cqlqashtjt7nda88awgwsc8wsg08c8hq66wka3"
        );
    }
}
