use sov_mock_da::{MockDaService, MockDaSpec, MockDaVerifier, MockHash};
use sov_mock_zkvm::{MockCodeCommitment, MockZkVerifier, MockZkvm, MockZkvmHost};
use sov_modules_api::ZkVerifier;
use sov_rollup_interface::zk::aggregated_proof::AggregatedProofPublicData;
use sov_stf_runner::processes::{
    ParallelProverService, ProofAggregationStatus, ProofProcessingStatus, ProverService,
    ProverServiceError, RollupProverConfigDiscriminants,
};

use super::{make_header, make_transition_info, wait_for_aggregated_proof, Address, StateRoot};
use crate::helpers::genesis_state_root;

struct TestProver {
    prover_service:
        ParallelProverService<Address, StateRoot, Vec<u8>, MockDaService, MockZkvm, MockZkvm>,
    inner_vm: MockZkvmHost,
    num_worker_threads: usize,
}

fn make_new_prover() -> TestProver {
    let num_threads = 10;
    let inner_vm = MockZkvmHost::new();
    let outer_vm = MockZkvmHost::new_non_blocking();

    let prover_config = RollupProverConfigDiscriminants::Prove;
    let da_verifier = MockDaVerifier::default();
    TestProver {
        prover_service: ParallelProverService::new(
            inner_vm.clone(),
            outer_vm,
            da_verifier,
            prover_config,
            num_threads,
            Default::default(),
        ),
        inner_vm,
        num_worker_threads: num_threads,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_successful_prover_execution() -> Result<(), ProverServiceError> {
    let TestProver {
        prover_service,
        inner_vm,
        ..
    } = make_new_prover();

    let header = make_header(MockHash::from([0; 32]), 1);
    prover_service
        .prove(make_transition_info(header.clone()))
        .await?;

    inner_vm.make_proof();

    let status =
        wait_for_aggregated_proof(&[header.clone()], &genesis_state_root(), &prover_service)
            .await
            .unwrap();

    assert!(matches!(status, ProofAggregationStatus::Success(_)));

    // The proof has already been sent, and the prover_service no longer has a reference to it.
    let err = prover_service
        .create_aggregated_proof(&[header], &genesis_state_root().0)
        .await
        .unwrap_err();

    assert_eq!(
        err.to_string(),
        "Missing required proof of 0x0000000000000000000000000000000000000000000000000000000000000000. Use the `prove` method to generate a proof of that block and try again."
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_prover_status_busy() -> anyhow::Result<()> {
    let TestProver {
        prover_service,
        inner_vm,
        num_worker_threads,
        ..
    } = make_new_prover();

    let genesis_state_root = genesis_state_root();

    let headers: Vec<_> = (1..num_worker_threads + 1)
        .map(|height| make_header(MockHash::from([height as u8; 32]), height as u64))
        .collect();

    // Saturate the prover.
    for header in &headers {
        let proof_processing_status = prover_service
            .prove(make_transition_info(header.clone()))
            .await?;
        assert!(matches!(
            proof_processing_status,
            ProofProcessingStatus::<Vec<u8>, Vec<u8>, MockDaSpec>::ProvingInProgress,
        ));

        let proof_submission_status = prover_service
            .create_aggregated_proof(std::slice::from_ref(header), &genesis_state_root.0)
            .await?;

        assert_eq!(
            ProofAggregationStatus::ProofGenerationInProgress,
            proof_submission_status
        );
    }

    // Attempting to create another proof while the prover is busy.
    {
        let header = make_header(MockHash::from([0; 32]), (num_worker_threads + 1) as u64);
        let status = prover_service
            .prove(make_transition_info(header.clone()))
            .await?;

        // The prover is busy and won't accept any new jobs.
        assert!(matches!(
            status,
            ProofProcessingStatus::<Vec<u8>, Vec<u8>, MockDaSpec>::Busy(_),
        ));

        let err = prover_service
            .create_aggregated_proof(&[header], &genesis_state_root.0)
            .await
            .unwrap_err();

        // The new job is not triggered.
        assert_eq!(
            err.to_string(),
            "Missing required proof of 0x0000000000000000000000000000000000000000000000000000000000000000. Use the `prove` method to generate a proof of that block and try again."
        );
    }

    for _ in 0..headers.len() {
        inner_vm.make_proof();
    }

    for header in &headers {
        let status = wait_for_aggregated_proof(
            std::slice::from_ref(header),
            &genesis_state_root,
            &prover_service,
        )
        .await
        .unwrap();
        assert!(matches!(status, ProofAggregationStatus::Success(_)));
    }

    // Retry once the prover is available to process new proofs.
    {
        let header = make_header(
            MockHash::from([(num_worker_threads + 1) as u8; 32]),
            (num_worker_threads + 2) as u64,
        );
        let status = prover_service.prove(make_transition_info(header)).await?;
        assert!(matches!(
            status,
            ProofProcessingStatus::<Vec<u8>, Vec<u8>, MockDaSpec>::ProvingInProgress
        ));
    }

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_generate_multiple_proofs_for_the_same_witness() -> anyhow::Result<()> {
    let TestProver { prover_service, .. } = make_new_prover();

    let header = make_header(MockHash::from([0; 32]), 1);

    let status = prover_service
        .prove(make_transition_info(header.clone()))
        .await?;
    assert!(matches!(
        status,
        ProofProcessingStatus::<Vec<u8>, Vec<u8>, MockDaSpec>::ProvingInProgress
    ));

    let err = prover_service
        .prove(make_transition_info(header))
        .await
        .expect_err(
            "Proof generation must fail when we try to prove the same block multiple times",
        );
    assert_eq!(err.to_string(), "Proof generation for 0x0000000000000000000000000000000000000000000000000000000000000000 still in progress");
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_aggregated_proof() -> Result<(), ProverServiceError> {
    let total_nb_of_blocks: usize = 10;
    let jump = 5;
    let end_block = jump + 1;

    let TestProver {
        prover_service,
        inner_vm,
        ..
    } = make_new_prover();

    let headers: Vec<_> = (0..total_nb_of_blocks)
        .map(|height| make_header(MockHash::from([height as u8; 32]), height as u64))
        .collect();

    let genesis_state_root = genesis_state_root();

    // Prove blocks form 0 to jump, where the number of submitted witnesses is equal to end_block.
    {
        for header in headers[0..end_block].iter().cloned() {
            prover_service.prove(make_transition_info(header)).await?;
        }

        let status =
            wait_for_aggregated_proof(&headers[0..jump], &genesis_state_root, &prover_service)
                .await
                .unwrap();
        // Waiting for the proof.
        assert!(matches!(
            status,
            ProofAggregationStatus::ProofGenerationInProgress
        ));

        // Make proof for each submitted block.
        for _ in 0..end_block {
            inner_vm.make_proof();
        }

        let status =
            wait_for_aggregated_proof(&headers[0..jump], &genesis_state_root, &prover_service)
                .await
                .unwrap();

        match status {
            ProofAggregationStatus::Success(proof) => {
                let public_data = <MockZkVerifier as ZkVerifier>::verify::<
                    AggregatedProofPublicData<Address, MockDaSpec, StateRoot>,
                >(
                    proof.raw_aggregated_proof.as_ref(),
                    &MockCodeCommitment::default(),
                )
                .unwrap();
                assert_eq!(public_data.initial_slot_number.get(), 0);
                assert_eq!(public_data.final_slot_number.get(), (jump - 1) as u64);
            }
            ProofAggregationStatus::ProofGenerationInProgress => panic!("Prover should succeed"),
        }
    }

    // Prove remaining blocks.
    {
        for header in headers[end_block..total_nb_of_blocks].iter().cloned() {
            prover_service.prove(make_transition_info(header)).await?;
            inner_vm.make_proof();
        }

        let status = wait_for_aggregated_proof(
            &headers[jump..total_nb_of_blocks],
            &genesis_state_root,
            &prover_service,
        )
        .await
        .unwrap();

        match status {
            ProofAggregationStatus::Success(proof) => {
                let public_data = <MockZkVerifier as ZkVerifier>::verify::<
                    AggregatedProofPublicData<Address, MockDaSpec, StateRoot>,
                >(
                    proof.raw_aggregated_proof.as_ref(),
                    &MockCodeCommitment::default(),
                )
                .unwrap();
                assert_eq!(public_data.initial_slot_number.get() as usize, jump);
                assert_eq!(
                    public_data.final_slot_number.get() as usize,
                    total_nb_of_blocks - 1
                );
            }
            ProofAggregationStatus::ProofGenerationInProgress => panic!("Proves should succeed"),
        }
    }

    Ok(())
}
