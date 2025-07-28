use sov_mock_da::{MockBlockHeader, MockDaService, MockDaSpec, MockDaVerifier, MockHash};
use sov_mock_zkvm::{MockCodeCommitment, MockZkVerifier, MockZkvm, MockZkvmHost};
use sov_modules_api::ZkVerifier;
use sov_rollup_interface::common::SlotNumber;
use sov_rollup_interface::da::{DaProof, RelevantBlobs, RelevantProofs, Time};
use sov_rollup_interface::zk::aggregated_proof::AggregatedProofPublicData;
use sov_rollup_interface::zk::StateTransitionWitness;
use sov_stf_runner::processes::{
    ParallelProverService, ProofAggregationStatus, ProofProcessingStatus, ProverService,
    ProverServiceError, RollupProverConfigDiscriminants, StateTransitionInfo,
};
use tokio::time;

use crate::helpers::{genesis_state_root, RawGenesisStateRoot};

type StateRoot = Vec<u8>;
type Address = Vec<u8>;

#[tokio::test(flavor = "multi_thread")]
async fn test_successful_prover_execution() -> Result<(), ProverServiceError> {
    let TestProver {
        prover_service,
        inner_vm,
        ..
    } = make_new_prover();

    let header_hash = MockHash::from([0; 32]);
    prover_service
        .prove(make_transition_info(header_hash, 1))
        .await?;

    inner_vm.make_proof();

    let status = wait_for_aggregated_proof(&[header_hash], &genesis_state_root(), &prover_service)
        .await
        .unwrap();

    assert!(matches!(status, ProofAggregationStatus::Success(_)));

    // The proof has already been sent, and the prover_service no longer has a reference to it.
    let err = prover_service
        .create_aggregated_proof(&[header_hash], &genesis_state_root().0)
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

    let header_hashes = (1..num_worker_threads + 1).map(|hash| MockHash::from([hash as u8; 32]));

    let mut height = 1;
    // Saturate the prover.
    for header_hash in header_hashes.clone() {
        let proof_processing_status = prover_service
            .prove(make_transition_info(header_hash, height))
            .await?;
        assert!(matches!(
            proof_processing_status,
            ProofProcessingStatus::<Vec<u8>, Vec<u8>, MockDaSpec>::ProvingInProgress,
        ));

        let proof_submission_status = prover_service
            .create_aggregated_proof(&[header_hash], &genesis_state_root.0)
            .await?;

        assert_eq!(
            ProofAggregationStatus::ProofGenerationInProgress,
            proof_submission_status
        );
        height += 1;
    }

    // Attempting to create another proof while the prover is busy.
    {
        let header_hash = MockHash::from([0; 32]);
        let status = prover_service
            .prove(make_transition_info(header_hash, height))
            .await?;

        height += 1;
        // The prover is busy and won't accept any new jobs.
        assert!(matches!(
            status,
            ProofProcessingStatus::<Vec<u8>, Vec<u8>, MockDaSpec>::Busy(_),
        ));

        let err = prover_service
            .create_aggregated_proof(&[header_hash], &genesis_state_root.0)
            .await
            .unwrap_err();

        // The new job is not triggered.
        assert_eq!(
            err.to_string(),
            "Missing required proof of 0x0000000000000000000000000000000000000000000000000000000000000000. Use the `prove` method to generate a proof of that block and try again."
        );
    }

    for _ in 0..header_hashes.len() {
        inner_vm.make_proof();
    }

    for header_hash in header_hashes.clone() {
        let status =
            wait_for_aggregated_proof(&[header_hash], &genesis_state_root, &prover_service)
                .await
                .unwrap();
        assert!(matches!(status, ProofAggregationStatus::Success(_)));
    }

    // Retry once the prover is available to process new proofs.
    {
        let header_hash = MockHash::from([(num_worker_threads + 1) as u8; 32]);
        let status = prover_service
            .prove(make_transition_info(header_hash, height))
            .await?;
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

    let header_hash = MockHash::from([0; 32]);

    let status = prover_service
        .prove(make_transition_info(header_hash, 1))
        .await?;
    assert!(matches!(
        status,
        ProofProcessingStatus::<Vec<u8>, Vec<u8>, MockDaSpec>::ProvingInProgress
    ));

    let err = prover_service
        .prove(make_transition_info(header_hash, 1))
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

    let header_hashes: Vec<_> = (0..total_nb_of_blocks)
        .map(|h| MockHash::from([h as u8; 32]))
        .collect();

    let genesis_state_root = genesis_state_root();

    // Prove blocks form 0 to jump, where the number of submitted witnesses is equal to end_block.
    {
        for (height, hash) in header_hashes[0..end_block].iter().enumerate() {
            prover_service
                .prove(make_transition_info(*hash, height as u64))
                .await?;
        }

        let status = wait_for_aggregated_proof(
            &header_hashes[0..jump],
            &genesis_state_root,
            &prover_service,
        )
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

        let status = wait_for_aggregated_proof(
            &header_hashes[0..jump],
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
                assert_eq!(public_data.initial_slot_number.get(), 0);
                assert_eq!(public_data.final_slot_number.get(), (jump - 1) as u64);
            }
            ProofAggregationStatus::ProofGenerationInProgress => panic!("Prover should succeed"),
        }
    }

    // Prove remaining blocks.
    {
        for (height, hash) in header_hashes[end_block..total_nb_of_blocks]
            .iter()
            .enumerate()
        {
            prover_service
                .prove(make_transition_info(*hash, (height + end_block) as u64))
                .await?;
            inner_vm.make_proof();
        }

        let status = wait_for_aggregated_proof(
            &header_hashes[jump..total_nb_of_blocks],
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

struct TestProver {
    prover_service:
        ParallelProverService<Address, StateRoot, Vec<u8>, MockDaService, MockZkvm, MockZkvm>,
    inner_vm: MockZkvmHost,
    num_worker_threads: usize,
}

async fn wait_for_aggregated_proof(
    header_hashes: &[MockHash],
    genesis_state_root: &RawGenesisStateRoot,
    prover_service: &ParallelProverService<
        Address,
        StateRoot,
        Vec<u8>,
        MockDaService,
        MockZkvm,
        MockZkvm,
    >,
) -> anyhow::Result<ProofAggregationStatus> {
    let mut counter = 0;
    loop {
        let status = prover_service
            .create_aggregated_proof(header_hashes, &genesis_state_root.0)
            .await?;

        if let ProofAggregationStatus::Success(_) = &status {
            return Ok(status);
        }

        if counter == 10 {
            return Ok(status);
        }

        time::sleep(time::Duration::from_millis(1000)).await;
        counter += 1;
    }
}

fn make_new_prover() -> TestProver {
    let num_threads = 10;
    let inner_vm = MockZkvmHost::new();
    let outer_vm = MockZkvmHost::new_non_blocking();

    let prover_config = RollupProverConfigDiscriminants::Execute;
    let da_verifier = MockDaVerifier::default();
    TestProver {
        prover_service: ParallelProverService::new(
            inner_vm.clone(),
            outer_vm,
            da_verifier,
            prover_config,
            num_threads,
            Default::default(),
            Default::default(),
        ),
        inner_vm,
        num_worker_threads: num_threads,
    }
}

fn make_transition_info(
    header_hash: MockHash,
    height: u64,
) -> StateTransitionInfo<StateRoot, Vec<u8>, MockDaSpec> {
    StateTransitionInfo::new(
        StateTransitionWitness {
            initial_state_root: Vec::default(),
            final_state_root: Vec::default(),
            da_block_header: MockBlockHeader {
                prev_hash: [0; 32].into(),
                hash: header_hash,
                height,
                time: Time::now(),
            },
            relevant_proofs: RelevantProofs {
                batch: DaProof {
                    inclusion_proof: Default::default(),
                    completeness_proof: Default::default(),
                },
                proof: DaProof {
                    inclusion_proof: Default::default(),
                    completeness_proof: Default::default(),
                },
            },
            relevant_blobs: RelevantBlobs {
                proof_blobs: vec![],
                batch_blobs: vec![],
            },
            witness: vec![],
        },
        SlotNumber::new_dangerous(height),
    )
}
