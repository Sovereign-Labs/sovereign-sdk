use std::time::Duration;

use sov_mock_da::{MockDaService, MockDaSpec};
use sov_modules_api::{AggregatedProofPublicData, Spec, Storage, ZkVerifier};
use sov_rollup_interface::common::SlotNumber;
use sov_sp1_adapter::host::{SP1AggregationHost, SP1Host};
use sov_sp1_adapter::{SP1MethodId, SP1Verifier, SP1};
use sov_stf_runner::processes::{
    ParallelProverService, ProofAggregationStatus, ProofProcessingStatus, ProverService,
    RollupProverConfigDiscriminants, StateTransitionInfo,
};

use super::{DefaultSpec, ProofStateRoot, ProofWitness};
use sov_rollup_interface::zk::ZkvmHost;

type TestParallelProverService = ParallelProverService<
    <DefaultSpec as Spec>::Address,
    ProofStateRoot,
    ProofWitness,
    MockDaService,
    SP1,
    SP1,
>;

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

    let elf: &[u8] = *sp1::SP1_GUEST_MOCK_ELF;
    assert!(
        !elf.is_empty(),
        "SP1 guest ELF is empty — build the guest first"
    );

    let inner_vm = tokio::task::spawn_blocking(move || SP1Host::new(elf).unwrap())
        .await
        .unwrap();

    let inner_vm_clone = inner_vm.clone();

    let code_commitment: SP1MethodId = tokio::task::spawn_blocking(move || -> SP1MethodId {
        inner_vm_clone
            .code_commitment()
            .expect("SP1 code commitment should be created successfully")
    })
    .await
    .unwrap();

    let agg_elf: &[u8] = *sp1::SP1_GUEST_AGGREGATION_MOCK_ELF;

    let outer_vm = tokio::task::spawn_blocking(move || {
        SP1AggregationHost::new(agg_elf, code_commitment).unwrap()
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
        RollupProverConfigDiscriminants::Prove,
        prover_address,
    );

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
    > = tokio::task::spawn_blocking(move || {
        SP1Verifier::verify(&status.raw_aggregated_proof, &outer_code_commitment).unwrap()
    })
    .await
    .unwrap();
}
