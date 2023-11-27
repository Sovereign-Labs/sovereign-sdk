use sov_mock_da::{MockBlockHeader, MockDaService, MockDaSpec, MockDaVerifier, MockValidityCond};
use sov_mock_zkvm::MockZkvm;
use sov_stf_runner::mock::MockStf;
use sov_stf_runner::{
    ParallelProverService, ProofProcessingStatus, ProofSubmissionStatus, ProverService,
    RollupProverConfig, StateTransitionData,
};

#[tokio::test]
async fn test_prover_prove() {
    let num_cpus = num_cpus::get();
    let vm = MockZkvm::default();
    let prover_service = make_new_prover(vm.clone(), num_cpus);

    let header_hash = [0; 32];
    prover_service
        .submit_witness(make_transition_data(header_hash))
        .await;

    prover_service.prove(header_hash).await.unwrap();
    vm.make_proof();

    for _ in 0..10 {
        let status = prover_service.send_proof_to_da(header_hash).await;
        if let Ok(ProofSubmissionStatus::Success) = status {
            return;
        }

        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await
    }

    panic!("Prover timed out")
}


#[tokio::test]
async fn test_prover_status_busy() -> Result<(), anyhow::Error> {
    let num_cpus = num_cpus::get();
    let vm = MockZkvm::default();
    let prover_service = make_new_prover(vm.clone(), num_cpus);

    for i in 0..num_cpus {
        let header_hash = [i as u8; 32];
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

    let header_hash = [(num_cpus + 1) as u8; 32];
    prover_service
        .submit_witness(make_transition_data(header_hash))
        .await;

    let status = prover_service.prove(header_hash).await?;
    assert_eq!(ProofProcessingStatus::Busy, status);

    let proof_submission_status = prover_service.send_proof_to_da(header_hash).await;
    // assert_eq!(ProofSubmissionStatus::ProvingInProgress, proof_submission_status);
    todo!();
    Ok(())
}


#[tokio::test]
async fn test_missing_witness() -> Result<(), anyhow::Error> {
    let num_cpus = num_cpus::get();
    let vm = MockZkvm::default();
    let prover_service = make_new_prover(vm.clone(), num_cpus);

    let header_hash = [0; 32];
    let status = prover_service.prove(header_hash).await;
    println!("{:?}", status);
    Ok(())
}

#[tokio::test]
async fn test_multiple_submissions() -> Result<(), anyhow::Error> {
    todo!();
    Ok(())
}

#[tokio::test]
async fn test_correct_execution() -> Result<(), anyhow::Error> {
    todo!();
    Ok(())
}


fn make_new_prover(
    vm: MockZkvm,
    num_threads: usize,
) -> ParallelProverService<[u8; 0], Vec<u8>, MockDaService, MockZkvm, MockStf<MockValidityCond>> {
    let prover_config = RollupProverConfig::Execute;
    let zk_stf = MockStf::<MockValidityCond>::default();
    let da_verifier = MockDaVerifier::default();
    ParallelProverService::new(
        vm.clone(),
        zk_stf,
        da_verifier,
        prover_config,
        (),
        num_threads,
    )
}

fn make_transition_data(
    header_hash: [u8; 32],
) -> StateTransitionData<[u8; 0], Vec<u8>, MockDaSpec> {
    StateTransitionData {
        pre_state_root: [],
        da_block_header: MockBlockHeader {
            prev_hash: [0; 32].into(),
            hash: header_hash.into(),
            height: 0,
        },
        inclusion_proof: [0; 32],
        completeness_proof: (),
        blobs: vec![],
        state_transition_witness: vec![],
    }
}
