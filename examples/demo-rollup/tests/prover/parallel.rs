use std::time::Duration;

use sov_mock_da::{MockDaService, MockDaSpec};
use sov_mock_zkvm::{MockZkvm, MockZkvmHost};
use sov_modules_api::Spec;
use sov_rollup_interface::common::SlotNumber;
use sov_rollup_interface::da::BlockHeaderTrait;
use sov_sp1_adapter::host::SP1Host;
use sov_sp1_adapter::SP1;
use sov_stf_runner::processes::{
    ParallelProverService, ProofAggregationStatus, ProofProcessingStatus, ProverService,
    RollupProverConfigDiscriminants, StateTransitionInfo,
};

use super::{DefaultSpec, ProofStateRoot, ProofWitness};

type TestParallelProverService = ParallelProverService<
    <DefaultSpec as Spec>::Address,
    ProofStateRoot,
    ProofWitness,
    MockDaService,
    SP1,
    MockZkvm,
>;

/// Tests proof generation using the SP1 parallel (local CPU) prover service.
///
/// This runs SP1 proofs locally using the parallel prover service for per-block.
///
/// Prerequisites:
///   - SP1 guest ELF built (`cargo build` in the prover guest directory)
///   - Sufficient CPU resources for local proving
#[tokio::test(flavor = "multi_thread")]
#[ignore = "Requires SP1 guest ELF and significant CPU resources for local proving"]
async fn test_parallel_proof_generation() {
    tracing_subscriber::fmt::init();

    let elf: &[u8] = *sp1::SP1_GUEST_MOCK_ELF;
    assert!(
        !elf.is_empty(),
        "SP1 guest ELF is empty — build the guest first"
    );

    let inner_vm = SP1Host::new(elf);
    // auto-complete outer proofs - real outer not supported yet
    let outer_vm = MockZkvmHost::new_non_blocking();

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
    let mut block_hashes = Vec::new();
    for (i, witness) in witnesses.into_iter().enumerate() {
        let block_header_hash = witness.da_block_header.hash();
        block_hashes.push(block_header_hash);

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
        block_hashes.len()
    );

    // Poll until the aggregated proof is ready.
    let status = loop {
        match prover_service
            .create_aggregated_proof(&block_hashes, &genesis_state_root)
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
}
