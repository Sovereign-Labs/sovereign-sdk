use std::time::Duration;

use sov_mock_da::{MockDaService, MockDaSpec};
use sov_modules_api::{AggregatedProofPublicData, Spec, Storage, ZkVerifier};
use sov_rollup_interface::common::SlotNumber;
use sov_sp1_adapter::host::{SP1AggregationHost, SP1Host};
use sov_sp1_adapter::{SP1Verifier, SP1};
use sov_stf_runner::processes::{
    ParallelProverService, ProofAggregationStatus, ProofProcessingStatus, ProverService,
    StateTransitionInfo,
};
use std::sync::Arc;

use super::{DefaultSpec, ProofStateRoot, ProofWitness};

type TestParallelProverService = ParallelProverService<
    <DefaultSpec as Spec>::Address,
    ProofStateRoot,
    ProofWitness,
    MockDaService,
    SP1,
    SP1,
>;

async fn make_parallel_prover_service() -> (TestParallelProverService, sov_sp1_adapter::SP1MethodId)
{
    let elf: &[u8] = *sp1::SP1_GUEST_MOCK_ELF;
    let agg_elf: &[u8] = *sp1::SP1_GUEST_AGGREGATION_MOCK_ELF;

    let outer_vk = tokio::task::spawn_blocking(move || {
        sov_sp1_adapter::host::verifying_key_from_elf(agg_elf).map(Arc::new)
    })
    .await
    .unwrap()
    .unwrap();

    let inner_vm = tokio::task::spawn_blocking(move || SP1Host::new(elf, outer_vk).unwrap())
        .await
        .unwrap();

    let inner_vm_clone = inner_vm.clone();

    let outer_vm = tokio::task::spawn_blocking(move || {
        SP1AggregationHost::new(agg_elf, inner_vm_clone.verifying_key().clone()).unwrap()
    })
    .await
    .unwrap();

    let outer_code_commitment = outer_vm.code_commitment();

    let da_verifier = sov_mock_da::MockDaVerifier::default();
    let prover_address = <DefaultSpec as Spec>::Address::try_from([0u8; 28].as_ref()).unwrap();

    let prover_service = TestParallelProverService::new_with_default_workers(
        inner_vm,
        outer_vm,
        da_verifier,
        prover_address,
    );

    (prover_service, outer_code_commitment)
}

/// Tests proof generation using the SP1 parallel (local CPU) prover service.
///
/// This runs SP1 proofs locally using the parallel prover service for per-block.
///
/// Prerequisites:
///   - SP1 guest ELF built (`cargo build` in the prover guest directory)
#[tokio::test(flavor = "multi_thread")]
async fn test_parallel_proof_generation() {
    // Use the mock prover: CPU proving is far too slow to run in tests.
    std::env::set_var("SP1_PROVER", "mock");

    let (prover_service, outer_code_commitment) = make_parallel_prover_service().await;

    let (genesis_state_root, witnesses) = super::generate_witnesses().await;

    // Submit all blocks to the parallel prover.
    let mut block_headers = Vec::new();
    for (i, witness) in witnesses.into_iter().enumerate() {
        block_headers.push(witness.da_block_header.clone());

        let slot_number = SlotNumber::new(i as u64 + 1);
        let state_transition_info = StateTransitionInfo::new(witness, slot_number);

        let status = prover_service
            .prove(state_transition_info)
            .await
            .expect("prove() should succeed");
        assert!(
            matches!(
                status,
                ProofProcessingStatus::<ProofStateRoot, ProofWitness, MockDaSpec>::ProvingInProgress
            ),
            "Expected ProvingInProgress after submitting block {i}"
        );
        tracing::info!("Block {} submitted to parallel prover", i);
    }

    tracing::info!(
        "All {} blocks submitted, waiting for proofs...",
        block_headers.len()
    );

    // Poll until the aggregated proof is ready.
    let status = loop {
        match prover_service
            .create_aggregated_proof(&block_headers, &genesis_state_root)
            .await
        {
            Ok(ProofAggregationStatus::Success(proof)) => break proof,
            Ok(ProofAggregationStatus::ProofGenerationInProgress) => {
                tracing::info!("Inner proofs still in progress, polling again in 5s...");
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
            Err(e) => panic!("Aggregation failed: {e}"),
        }
    };

    tracing::info!(
        "Aggregated proof generated successfully ({} bytes)",
        status.raw_aggregated_proof.len()
    );
    assert!(
        !status.raw_aggregated_proof.is_empty(),
        "Aggregated proof should not be empty"
    );

    let _public_data: AggregatedProofPublicData<
        <DefaultSpec as Spec>::Address,
        MockDaSpec,
        <<DefaultSpec as Spec>::Storage as Storage>::Root,
    > = SP1Verifier::verify_with_proof(&status.to_serialized_zk_proof(), &outer_code_commitment)
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_parallel_proof_generation_restores_outer_continuity_after_restart() {
    std::env::set_var("SP1_PROVER", "mock");
    std::env::set_var("SOV_BENCH_BLOCKS", "3");

    let split = 2usize;
    let (genesis_state_root, witnesses) = super::generate_witnesses().await;
    let mut witnesses = witnesses.into_iter();
    let first_batch: Vec<_> = witnesses.by_ref().take(split).collect();
    let second_batch: Vec<_> = witnesses.collect();
    assert!(
        !second_batch.is_empty(),
        "test requires more than one aggregation batch"
    );

    let (first_prover_service, _) = make_parallel_prover_service().await;

    let mut first_block_headers = Vec::new();
    for (i, witness) in first_batch.into_iter().enumerate() {
        first_block_headers.push(witness.da_block_header.clone());

        let slot_number = SlotNumber::new(i as u64 + 1);
        let state_transition_info = StateTransitionInfo::new(witness, slot_number);
        let status = first_prover_service
            .prove(state_transition_info)
            .await
            .expect("prove() should succeed");
        assert!(matches!(
            status,
            ProofProcessingStatus::<ProofStateRoot, ProofWitness, MockDaSpec>::ProvingInProgress
        ));
    }

    let restored_outer_proof = loop {
        match first_prover_service
            .create_aggregated_proof(&first_block_headers, &genesis_state_root)
            .await
        {
            Ok(ProofAggregationStatus::Success(proof)) => break proof,
            Ok(ProofAggregationStatus::ProofGenerationInProgress) => {
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            Err(e) => panic!("First aggregation failed: {e}"),
        }
    };

    let (second_prover_service, outer_code_commitment) = make_parallel_prover_service().await;
    second_prover_service
        .restore_persisted_aggregated_proof(restored_outer_proof)
        .expect("restoring the last aggregated proof should succeed");

    let mut second_block_headers = Vec::new();
    for (i, witness) in second_batch.into_iter().enumerate() {
        second_block_headers.push(witness.da_block_header.clone());

        let slot_number = SlotNumber::new((split + i) as u64 + 1);
        let state_transition_info = StateTransitionInfo::new(witness, slot_number);
        let status = second_prover_service
            .prove(state_transition_info)
            .await
            .expect("prove() should succeed");
        assert!(matches!(
            status,
            ProofProcessingStatus::<ProofStateRoot, ProofWitness, MockDaSpec>::ProvingInProgress
        ));
    }

    let status = loop {
        match second_prover_service
            .create_aggregated_proof(&second_block_headers, &genesis_state_root)
            .await
        {
            Ok(ProofAggregationStatus::Success(proof)) => break proof,
            Ok(ProofAggregationStatus::ProofGenerationInProgress) => {
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            Err(e) => panic!("Second aggregation failed: {e}"),
        }
    };

    let public_data: AggregatedProofPublicData<
        <DefaultSpec as Spec>::Address,
        MockDaSpec,
        <<DefaultSpec as Spec>::Storage as Storage>::Root,
    > = SP1Verifier::verify_with_proof(&status.to_serialized_zk_proof(), &outer_code_commitment)
        .unwrap();
    assert_eq!(public_data.initial_slot_number.get(), split as u64 + 1);
    assert_eq!(
        public_data.final_slot_number.get(),
        (split + second_block_headers.len()) as u64
    );
    assert_eq!(public_data.genesis_state_root, genesis_state_root);
}
