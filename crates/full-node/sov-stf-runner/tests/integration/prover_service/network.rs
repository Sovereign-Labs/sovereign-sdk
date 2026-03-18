use sov_mock_da::{MockDaService, MockDaSpec, MockDaVerifier, MockHash};
use sov_mock_zkvm::{MockCodeCommitment, MockZkVerifier, MockZkvm, MockZkvmNetwork};
use sov_modules_api::ZkVerifier;
use sov_rollup_interface::zk::aggregated_proof::AggregatedProofPublicData;
use sov_stf_runner::processes::{
    NetworkProverService, ProofAggregationStatus, ProofProcessingStatus, ProverService,
};

use super::{make_transition_info, Address, StateRoot};
use crate::helpers::genesis_state_root;

struct TestNetworkProver {
    prover_service:
        NetworkProverService<Address, StateRoot, Vec<u8>, MockDaService, MockZkvm, MockZkvm>,
    inner_vm: MockZkvmNetwork,
}

fn make_network_prover(inner_auto_complete: bool, outer_auto_complete: bool) -> TestNetworkProver {
    let inner_vm = MockZkvmNetwork::new(inner_auto_complete);
    let outer_vm = MockZkvmNetwork::new(outer_auto_complete);

    let da_verifier = MockDaVerifier::default();
    TestNetworkProver {
        prover_service: NetworkProverService::new(
            inner_vm.clone(),
            outer_vm,
            da_verifier,
            Default::default(),
            vec![],
        ),
        inner_vm,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_network_prove_and_aggregate() {
    // Outer auto-completes so the poll loop in create_aggregated_proof returns immediately.
    let TestNetworkProver {
        prover_service,
        inner_vm,
    } = make_network_prover(false, true);

    let header_hash = MockHash::from([1; 32]);
    let genesis = genesis_state_root();

    // Submit one block — should return ProvingInProgress.
    let status = prover_service
        .prove(make_transition_info(header_hash, 1))
        .await
        .unwrap();
    assert!(matches!(
        status,
        ProofProcessingStatus::<Vec<u8>, Vec<u8>, MockDaSpec>::ProvingInProgress,
    ));

    // Inner proof not ready yet → ProofGenerationInProgress.
    let status = prover_service
        .create_aggregated_proof(&[header_hash], &genesis.0)
        .await
        .unwrap();
    assert_eq!(status, ProofAggregationStatus::ProofGenerationInProgress);

    // Complete inner proof (handle 0 is the first submitted proof).
    inner_vm.complete_proof(0);

    // Now aggregation should succeed.
    let status = prover_service
        .create_aggregated_proof(&[header_hash], &genesis.0)
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
            assert_eq!(public_data.initial_slot_number.get(), 1);
            assert_eq!(public_data.final_slot_number.get(), 1);
        }
        ProofAggregationStatus::ProofGenerationInProgress => {
            panic!("Expected Success after completing inner proof")
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_network_prove_returns_in_progress() {
    let TestNetworkProver {
        prover_service,
        inner_vm: _,
    } = make_network_prover(false, true);

    let header_hash = MockHash::from([2; 32]);
    let genesis = genesis_state_root();

    prover_service
        .prove(make_transition_info(header_hash, 1))
        .await
        .unwrap();

    // Don't complete the inner proof — aggregation should stay in progress.
    let status = prover_service
        .create_aggregated_proof(&[header_hash], &genesis.0)
        .await
        .unwrap();
    assert_eq!(status, ProofAggregationStatus::ProofGenerationInProgress);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_network_aggregated_proof_multiple_blocks() {
    let TestNetworkProver {
        prover_service,
        inner_vm,
    } = make_network_prover(false, true);

    let block_count = 5;
    let genesis = genesis_state_root();
    let hashes: Vec<_> = (0..block_count).map(|i| MockHash::from([i + 10; 32])).collect();

    // Submit all blocks.
    for (i, hash) in hashes.iter().enumerate() {
        prover_service
            .prove(make_transition_info(*hash, i as u64))
            .await
            .unwrap();
    }

    // Complete all inner proofs (handles 0..5).
    for handle in 0..block_count {
        inner_vm.complete_proof(handle.into());
    }

    let status = prover_service
        .create_aggregated_proof(&hashes, &genesis.0)
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
            assert_eq!(public_data.final_slot_number.get(), 4);
        }
        ProofAggregationStatus::ProofGenerationInProgress => {
            panic!("Expected Success after completing all inner proofs")
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_network_duplicate_proof_rejected() {
    let TestNetworkProver {
        prover_service,
        inner_vm: _,
    } = make_network_prover(false, true);

    let header_hash = MockHash::from([4; 32]);

    // First prove succeeds.
    let status = prover_service
        .prove(make_transition_info(header_hash, 1))
        .await
        .unwrap();
    assert!(matches!(
        status,
        ProofProcessingStatus::<Vec<u8>, Vec<u8>, MockDaSpec>::ProvingInProgress,
    ));

    // Second prove with same header hash should fail.
    let err = prover_service
        .prove(make_transition_info(header_hash, 1))
        .await
        .expect_err("Duplicate proof submission should be rejected");
    assert_eq!(
        err.to_string(),
        format!("Proof generation for {} still in progress", header_hash)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_network_prove_rejected_after_proved() {
    let TestNetworkProver {
        prover_service,
        inner_vm,
    } = make_network_prover(false, true);

    let genesis = genesis_state_root();
    let hash_a = MockHash::from([5; 32]);
    let hash_b = MockHash::from([6; 32]);

    // Submit two blocks.
    prover_service
        .prove(make_transition_info(hash_a, 1))
        .await
        .unwrap();
    prover_service
        .prove(make_transition_info(hash_b, 2))
        .await
        .unwrap();

    // Complete only block A's inner proof (handle 0).
    inner_vm.complete_proof(0);

    // Aggregate [A, B]: A transitions to Proved, B is still pending → InProgress.
    let status = prover_service
        .create_aggregated_proof(&[hash_a, hash_b], &genesis.0)
        .await
        .unwrap();
    assert_eq!(status, ProofAggregationStatus::ProofGenerationInProgress);

    // Proving A again should fail because it is already Proved.
    let err = prover_service
        .prove(make_transition_info(hash_a, 1))
        .await
        .expect_err("Re-proving a Proved block should be rejected");
    assert_eq!(
        err.to_string(),
        format!(
            "Witness for block_header_hash {}, submitted multiple times.",
            hash_a
        )
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_network_prove_rejected_after_error() {
    let TestNetworkProver {
        prover_service,
        inner_vm,
    } = make_network_prover(false, true);

    let genesis = genesis_state_root();
    let hash_a = MockHash::from([7; 32]);

    // Submit block A.
    prover_service
        .prove(make_transition_info(hash_a, 1))
        .await
        .unwrap();

    // Remove the proof so poll returns an error.
    inner_vm.fail_proof(0);

    // Aggregation triggers the Err branch in Phase 1.
    let err = prover_service
        .create_aggregated_proof(&[hash_a], &genesis.0)
        .await
        .expect_err("Aggregation should fail when proof is missing");
    assert!(
        err.to_string().contains("unknown proof handle: 0"),
        "Expected 'unknown proof handle' error, got: {}",
        err
    );

    // Proving A again should propagate the stored error.
    let err = prover_service
        .prove(make_transition_info(hash_a, 1))
        .await
        .expect_err("Re-proving after error should propagate the stored error");
    assert!(
        err.to_string().contains("unknown proof handle: 0"),
        "Expected stored error to contain 'unknown proof handle', got: {}",
        err
    );
}
