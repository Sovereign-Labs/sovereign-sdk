#[cfg(test)]
mod test {
    use sov_app_template::Batch;
    use sov_modules_api::mocks::MockContext;
    use sov_state::ProverStorage;
    use sovereign_sdk::stf::StateTransitionFunction;

    use crate::{
        data_generation::{simulate_da, QueryGenerator},
        helpers::check_query,
        runtime::Runtime,
        test_utils::{create_new_demo, C, LOCKED_AMOUNT, SEQUENCER_DA_ADDRESS},
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
            check_query(
                runtime,
                QueryGenerator::generate_query_election_message(),
                r#"{"Result":{"name":"candidate_2","count":3}}"#,
                storage.clone(),
            );

            check_query(
                runtime,
                QueryGenerator::generate_query_value_setter_message(),
                r#"{"value":33}"#,
                storage,
            );
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
        check_query(
            runtime,
            QueryGenerator::generate_query_election_message(),
            r#"{"Result":{"name":"candidate_2","count":3}}"#,
            demo.current_storage.clone(),
        );

        check_query(
            runtime,
            QueryGenerator::generate_query_value_setter_message(),
            r#"{"value":33}"#,
            demo.current_storage,
        );
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
            check_query(
                runtime,
                QueryGenerator::generate_query_election_message(),
                r#"{"Err":"Election is not frozen"}"#,
                storage.clone(),
            );

            check_query(
                runtime,
                QueryGenerator::generate_query_value_setter_message(),
                r#"{"value":null}"#,
                storage,
            );
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
