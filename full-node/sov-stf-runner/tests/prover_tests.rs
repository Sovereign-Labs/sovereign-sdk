use sov_mock_da::{
    MockBlockHeader, MockDaService, MockDaSpec, MockDaVerifier, MockHash, MockValidityCond,
};
use sov_mock_zkvm::MockZkvm;
use sov_rollup_interface::da::Time;
use sov_rollup_interface::zk::StateTransitionData;
use sov_stf_runner::mock::MockStf;
use sov_stf_runner::{
    ParallelProverService, ProofProcessingStatus, ProofSubmissionStatus, ProverService,
    ProverServiceConfig, ProverServiceError, RollupProverConfig, WitnessSubmissionStatus,
};

#[tokio::test]
async fn test_successful_prover_execution() -> Result<(), ProverServiceError> {
    let TestProver {
        prover_service, vm, ..
    } = make_new_prover();

    let header_hash = MockHash::from([0; 32]);
    prover_service
        .submit_witness(make_transition_data(header_hash))
        .await;
    prover_service.prove(header_hash).await?;
    vm.make_proof();
    wait_for_proof_proof_da_submission(header_hash, &prover_service).await;

    // The proof has already been sent, and the prover_service no longer has a reference to it.
    let err = prover_service
        .send_proof_to_da(header_hash)
        .await
        .unwrap_err();

    assert_eq!(
        err.to_string(),
        "Missing witness for: 0x0000000000000000000000000000000000000000000000000000000000000000"
    );

    Ok(())
}

#[tokio::test]
async fn test_prover_status_busy() -> Result<(), anyhow::Error> {
    let TestProver {
        prover_service,
        vm,
        num_worker_threads,
        ..
    } = make_new_prover();

    let header_hashes = (1..num_worker_threads + 1).map(|hash| MockHash::from([hash as u8; 32]));

    // Saturate the prover.
    for header_hash in header_hashes.clone() {
        prover_service
            .submit_witness(make_transition_data(header_hash))
            .await;

        let poof_processing_status = prover_service.prove(header_hash).await?;
        assert_eq!(
            ProofProcessingStatus::ProvingInProgress,
            poof_processing_status
        );

        let proof_submission_status = prover_service.send_proof_to_da(header_hash).await?;
        assert_eq!(
            ProofSubmissionStatus::ProofGenerationInProgress,
            proof_submission_status
        );
    }

    // Attempting to create another proof while the prover is busy.
    {
        let header_hash = MockHash::from([0; 32]);
        prover_service
            .submit_witness(make_transition_data(header_hash))
            .await;

        let status = prover_service.prove(header_hash).await?;
        // The prover is busy and won't accept any new jobs.
        assert_eq!(ProofProcessingStatus::Busy, status);

        let proof_submission_status = prover_service
            .send_proof_to_da(header_hash)
            .await
            .unwrap_err();

        // The new job wasn't accepted.
        assert_eq!(
        proof_submission_status.to_string(),
        "Missing witness for: 0x0000000000000000000000000000000000000000000000000000000000000000");
    }

    vm.make_proof();
    for header_hash in header_hashes.clone() {
        wait_for_proof_proof_da_submission(header_hash, &prover_service).await;
    }

    // Retry once the prover is available to process new proofs.
    {
        let header_hash = MockHash::from([(num_worker_threads + 1) as u8; 32]);
        prover_service
            .submit_witness(make_transition_data(header_hash))
            .await;

        let status = prover_service.prove(header_hash).await?;
        assert_eq!(ProofProcessingStatus::ProvingInProgress, status);
    }

    Ok(())
}

#[tokio::test]
async fn test_missing_witness() -> Result<(), anyhow::Error> {
    let TestProver { prover_service, .. } = make_new_prover();
    let header_hash = MockHash::from([0; 32]);
    let err = prover_service.prove(header_hash).await.unwrap_err();

    assert_eq!(
        err.to_string(),
        "Missing witness for block: 0x0000000000000000000000000000000000000000000000000000000000000000"
    );
    Ok(())
}

#[tokio::test]
async fn test_multiple_witness_submissions() -> Result<(), anyhow::Error> {
    let TestProver { prover_service, .. } = make_new_prover();

    let header_hash = MockHash::from([0; 32]);
    let submission_status = prover_service
        .submit_witness(make_transition_data(header_hash))
        .await;

    assert_eq!(
        WitnessSubmissionStatus::SubmittedForProving,
        submission_status
    );

    let submission_status = prover_service
        .submit_witness(make_transition_data(header_hash))
        .await;

    assert_eq!(WitnessSubmissionStatus::WitnessExist, submission_status);

    Ok(())
}

#[tokio::test]
async fn test_generate_multiple_proofs_for_the_same_witness() -> Result<(), anyhow::Error> {
    let TestProver { prover_service, .. } = make_new_prover();

    let header_hash = MockHash::from([0; 32]);
    prover_service
        .submit_witness(make_transition_data(header_hash))
        .await;

    let status = prover_service.prove(header_hash).await?;
    assert_eq!(ProofProcessingStatus::ProvingInProgress, status);

    let err = prover_service.prove(header_hash).await.unwrap_err();
    assert_eq!(err.to_string(), "Proof generation for 0x0000000000000000000000000000000000000000000000000000000000000000 still in progress");
    Ok(())
}

struct TestProver {
    prover_service: ParallelProverService<
        [u8; 0],
        Vec<u8>,
        MockDaService,
        MockZkvm<MockValidityCond>,
        MockStf<MockValidityCond>,
    >,
    vm: MockZkvm<MockValidityCond>,
    num_worker_threads: usize,
}

async fn wait_for_proof_proof_da_submission(
    header_hash: MockHash,
    prover_service: &ParallelProverService<
        [u8; 0],
        Vec<u8>,
        MockDaService,
        MockZkvm<MockValidityCond>,
        MockStf<MockValidityCond>,
    >,
) {
    for _ in 0..10 {
        let status = prover_service.send_proof_to_da(header_hash).await;
        if let Ok(ProofSubmissionStatus::Success) = status {
            return;
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await
    }
}

fn make_new_prover() -> TestProver {
    let num_threads = num_cpus::get();
    let vm = MockZkvm::new(MockValidityCond::default());

    let prover_config = RollupProverConfig::Execute;
    let zk_stf = MockStf::<MockValidityCond>::default();
    let da_verifier = MockDaVerifier::default();
    TestProver {
        prover_service: ParallelProverService::new(
            vm.clone(),
            zk_stf,
            da_verifier,
            prover_config,
            (),
            num_threads,
            ProverServiceConfig {
                aggregated_proof_block_jump: 1,
            },
        ),
        vm,
        num_worker_threads: num_threads,
    }
}

fn make_transition_data(
    header_hash: MockHash,
) -> StateTransitionData<[u8; 0], Vec<u8>, MockDaSpec> {
    StateTransitionData {
        initial_state_root: [],
        final_state_root: [],
        da_block_header: MockBlockHeader {
            prev_hash: [0; 32].into(),
            hash: header_hash,
            height: 0,
            time: Time::now(),
        },
        inclusion_proof: [0; 32],
        completeness_proof: (),
        blobs: vec![],
        state_transition_witness: vec![],
    }
}
