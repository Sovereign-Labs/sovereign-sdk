#[cfg(test)]
pub mod test {
    use sov_modules_api::default_context::DefaultContext;
    use sov_modules_api::default_signature::private_key::DefaultPrivateKey;
    use sov_modules_stf_template::{Batch, SequencerOutcome};
    use sov_rollup_interface::mocks::MockZkvm;
    use sov_rollup_interface::stf::StateTransitionFunction;
    use sov_state::{ProverStorage, WorkingSet};

    use crate::genesis_config::{create_demo_config, DEMO_SEQUENCER_DA_ADDRESS, LOCKED_AMOUNT};
    use crate::runtime::Runtime;
    use crate::tests::data_generation::simulate_da;
    use crate::tests::{create_new_demo, has_tx_events, new_test_blob, C};

    #[test]
    fn test_demo_values_in_db() {
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path();
        let value_setter_admin_private_key = DefaultPrivateKey::generate();
        let election_admin_private_key = DefaultPrivateKey::generate();

        let config = create_demo_config(
            LOCKED_AMOUNT + 1,
            &value_setter_admin_private_key,
            &election_admin_private_key,
        );
        {
            let mut demo = create_new_demo(path);

            StateTransitionFunction::<MockZkvm>::init_chain(&mut demo, config);
            StateTransitionFunction::<MockZkvm>::begin_slot(&mut demo, Default::default());

            let txs = simulate_da(value_setter_admin_private_key, election_admin_private_key);

            let apply_blob_outcome = StateTransitionFunction::<MockZkvm>::apply_blob(
                &mut demo,
                &mut new_test_blob(Batch { txs }, &DEMO_SEQUENCER_DA_ADDRESS),
                None,
            );

            assert_eq!(
                SequencerOutcome::Rewarded(0),
                apply_blob_outcome.inner,
                "Sequencer execution should have succeeded but failed "
            );

            assert!(has_tx_events(&apply_blob_outcome),);

            StateTransitionFunction::<MockZkvm>::end_slot(&mut demo);
        }

        // Generate a new storage instance after dumping data to the db.
        {
            let runtime = &mut Runtime::<DefaultContext>::default();
            let storage = ProverStorage::with_path(path).unwrap();
            let mut working_set = WorkingSet::new(storage);

            let resp = runtime.election.results(&mut working_set);

            assert_eq!(
                resp,
                sov_election::query::GetResultResponse::Result(Some(sov_election::Candidate {
                    name: "candidate_2".to_owned(),
                    count: 3
                }))
            );
            let resp = runtime.value_setter.query_value(&mut working_set);

            assert_eq!(resp, sov_value_setter::query::Response { value: Some(33) });
        }
    }

    #[test]
    fn test_demo_values_in_cache() {
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path();
        let mut demo = create_new_demo(path);

        let value_setter_admin_private_key = DefaultPrivateKey::generate();
        let election_admin_private_key = DefaultPrivateKey::generate();

        let config = create_demo_config(
            LOCKED_AMOUNT + 1,
            &value_setter_admin_private_key,
            &election_admin_private_key,
        );

        StateTransitionFunction::<MockZkvm>::init_chain(&mut demo, config);
        StateTransitionFunction::<MockZkvm>::begin_slot(&mut demo, Default::default());

        let txs = simulate_da(value_setter_admin_private_key, election_admin_private_key);

        let apply_blob_outcome = StateTransitionFunction::<MockZkvm>::apply_blob(
            &mut demo,
            &mut new_test_blob(Batch { txs }, &DEMO_SEQUENCER_DA_ADDRESS),
            None,
        );

        assert_eq!(
            SequencerOutcome::Rewarded(0),
            apply_blob_outcome.inner,
            "Sequencer execution should have succeeded but failed"
        );

        assert!(has_tx_events(&apply_blob_outcome),);

        StateTransitionFunction::<MockZkvm>::end_slot(&mut demo);

        let runtime = &mut Runtime::<DefaultContext>::default();
        let mut working_set = WorkingSet::new(demo.current_storage.clone());

        let resp = runtime.election.results(&mut working_set);

        assert_eq!(
            resp,
            sov_election::query::GetResultResponse::Result(Some(sov_election::Candidate {
                name: "candidate_2".to_owned(),
                count: 3
            }))
        );

        let resp = runtime.value_setter.query_value(&mut working_set);

        assert_eq!(resp, sov_value_setter::query::Response { value: Some(33) });
    }

    #[test]
    fn test_demo_values_not_in_db() {
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path();

        let value_setter_admin_private_key = DefaultPrivateKey::generate();
        let election_admin_private_key = DefaultPrivateKey::generate();

        let config = create_demo_config(
            LOCKED_AMOUNT + 1,
            &value_setter_admin_private_key,
            &election_admin_private_key,
        );
        {
            let mut demo = create_new_demo(path);

            StateTransitionFunction::<MockZkvm>::init_chain(&mut demo, config);
            StateTransitionFunction::<MockZkvm>::begin_slot(&mut demo, Default::default());

            let txs = simulate_da(value_setter_admin_private_key, election_admin_private_key);

            let apply_blob_outcome = StateTransitionFunction::<MockZkvm>::apply_blob(
                &mut demo,
                &mut new_test_blob(Batch { txs }, &DEMO_SEQUENCER_DA_ADDRESS),
                None,
            )
            .inner;
            assert_eq!(
                SequencerOutcome::Rewarded(0),
                apply_blob_outcome,
                "Sequencer execution should have succeeded but failed",
            );
        }

        // Generate a new storage instance, value are missing because we didn't call `end_slot()`;
        {
            let runtime = &mut Runtime::<C>::default();
            let storage = ProverStorage::with_path(path).unwrap();
            let mut working_set = WorkingSet::new(storage);

            let resp = runtime.election.results(&mut working_set);

            assert_eq!(
                resp,
                sov_election::query::GetResultResponse::Err("Election is not frozen".to_owned())
            );

            let resp = runtime.value_setter.query_value(&mut working_set);

            assert_eq!(resp, sov_value_setter::query::Response { value: None });
        }
    }

    #[test]
    fn test_sequencer_unknown_sequencer() {
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path();

        let value_setter_admin_private_key = DefaultPrivateKey::generate();
        let election_admin_private_key = DefaultPrivateKey::generate();

        let config = create_demo_config(
            LOCKED_AMOUNT + 1,
            &value_setter_admin_private_key,
            &election_admin_private_key,
        );

        let mut demo = create_new_demo(path);

        StateTransitionFunction::<MockZkvm>::init_chain(&mut demo, config);
        StateTransitionFunction::<MockZkvm>::begin_slot(&mut demo, Default::default());

        let txs = simulate_da(value_setter_admin_private_key, election_admin_private_key);

        let some_sequencer: [u8; 32] = [121; 32];
        let apply_blob_outcome = StateTransitionFunction::<MockZkvm>::apply_blob(
            &mut demo,
            &mut new_test_blob(Batch { txs }, &some_sequencer),
            None,
        );

        assert_eq!(
            SequencerOutcome::Ignored,
            apply_blob_outcome.inner,
            "Batch should have been skipped due to unknown sequencer"
        );

        // Assert that there are no events
        assert!(!has_tx_events(&apply_blob_outcome));
    }
}
