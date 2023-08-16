#[cfg(test)]
pub mod test {

    use sov_data_generators::{has_tx_events, new_test_blob_from_batch};
    use sov_modules_api::default_context::DefaultContext;
    use sov_modules_api::default_signature::private_key::DefaultPrivateKey;
    use sov_modules_api::PrivateKey;
    use sov_modules_stf_template::{Batch, SequencerOutcome};
    use sov_rollup_interface::mocks::{MockBlock, MockZkvm};
    use sov_rollup_interface::stf::StateTransitionFunction;
    use sov_state::{ProverStorage, WorkingSet};

    use crate::genesis_config::{create_demo_config, DEMO_SEQUENCER_DA_ADDRESS, LOCKED_AMOUNT};
    use crate::runtime::Runtime;
    use crate::tests::da_simulation::simulate_da;
    use crate::tests::{create_new_demo, TestBlob, C};

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

            StateTransitionFunction::<MockZkvm, TestBlob>::init_chain(&mut demo, config);

            let txs = simulate_da(value_setter_admin_private_key, election_admin_private_key);
            let blob = new_test_blob_from_batch(Batch { txs }, &DEMO_SEQUENCER_DA_ADDRESS, [0; 32]);

            let mut blobs = [blob];

            let data = MockBlock::default();

            let result = StateTransitionFunction::<MockZkvm, TestBlob>::apply_slot(
                &mut demo,
                Default::default(),
                &data,
                &mut blobs,
            );

            assert_eq!(1, result.batch_receipts.len());
            let apply_blob_outcome = result.batch_receipts[0].clone();
            assert_eq!(
                SequencerOutcome::Rewarded(0),
                apply_blob_outcome.inner,
                "Sequencer execution should have succeeded but failed "
            );

            assert!(has_tx_events(&apply_blob_outcome),);
        }

        // Generate a new storage instance after dumping data to the db.
        {
            let runtime = &mut Runtime::<DefaultContext>::default();
            let storage = ProverStorage::with_path(path).unwrap();
            let mut working_set = WorkingSet::new(storage);

            let resp = runtime.election.results(&mut working_set).unwrap();

            assert_eq!(
                resp,
                sov_election::GetResultResponse::Result(Some(sov_election::Candidate {
                    name: "candidate_2".to_owned(),
                    count: 3
                }))
            );
            let resp = runtime.value_setter.query_value(&mut working_set).unwrap();

            assert_eq!(resp, sov_value_setter::Response { value: Some(33) });
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

        StateTransitionFunction::<MockZkvm, TestBlob>::init_chain(&mut demo, config);

        let txs = simulate_da(value_setter_admin_private_key, election_admin_private_key);

        let blob = new_test_blob_from_batch(Batch { txs }, &DEMO_SEQUENCER_DA_ADDRESS, [0; 32]);
        let mut blobs = [blob];
        let data = MockBlock::default();

        let apply_block_result = StateTransitionFunction::<MockZkvm, TestBlob>::apply_slot(
            &mut demo,
            Default::default(),
            &data,
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

        let runtime = &mut Runtime::<DefaultContext>::default();
        let mut working_set = WorkingSet::new(demo.current_storage.clone());

        let resp = runtime.election.results(&mut working_set).unwrap();

        assert_eq!(
            resp,
            sov_election::GetResultResponse::Result(Some(sov_election::Candidate {
                name: "candidate_2".to_owned(),
                count: 3
            }))
        );

        let resp = runtime.value_setter.query_value(&mut working_set).unwrap();

        assert_eq!(resp, sov_value_setter::Response { value: Some(33) });
    }

    #[test]
    #[ignore = "end_slot is removed from STF trait"]
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

            StateTransitionFunction::<MockZkvm, TestBlob>::init_chain(&mut demo, config);

            let txs = simulate_da(value_setter_admin_private_key, election_admin_private_key);
            let blob = new_test_blob_from_batch(Batch { txs }, &DEMO_SEQUENCER_DA_ADDRESS, [0; 32]);
            let mut blobs = [blob];
            let data = MockBlock::default();

            let apply_block_result = StateTransitionFunction::<MockZkvm, TestBlob>::apply_slot(
                &mut demo,
                Default::default(),
                &data,
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

        // Generate a new storage instance, value are missing because we didn't call `end_slot()`;
        {
            let runtime = &mut Runtime::<C>::default();
            let storage = ProverStorage::with_path(path).unwrap();
            let mut working_set = WorkingSet::new(storage);

            let resp = runtime.election.results(&mut working_set).unwrap();

            assert_eq!(
                resp,
                sov_election::GetResultResponse::Err("Election is not frozen".to_owned())
            );

            let resp = runtime.value_setter.query_value(&mut working_set).unwrap();

            assert_eq!(resp, sov_value_setter::Response { value: None });
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

        StateTransitionFunction::<MockZkvm, TestBlob>::init_chain(&mut demo, config);

        let some_sequencer: [u8; 32] = [121; 32];
        let txs = simulate_da(value_setter_admin_private_key, election_admin_private_key);
        let blob = new_test_blob_from_batch(Batch { txs }, &some_sequencer, [0; 32]);
        let mut blobs = [blob];
        let data = MockBlock::default();

        let apply_block_result = StateTransitionFunction::<MockZkvm, TestBlob>::apply_slot(
            &mut demo,
            Default::default(),
            &data,
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
}
