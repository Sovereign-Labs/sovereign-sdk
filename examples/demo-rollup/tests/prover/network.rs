use std::time::Duration;

use sov_mock_da::{MockDaService, MockDaSpec};
use sov_mock_zkvm::{MockZkvm, MockZkvmNetwork};
use sov_modules_api::Spec;
use sov_rollup_interface::common::SlotNumber;
use sov_rollup_interface::da::BlockHeaderTrait;
use sov_sp1_adapter::network::SP1Network;
use sov_sp1_adapter::SP1;
use sov_stf_runner::processes::{
    NetworkProverService, ProofAggregationStatus, ProofProcessingStatus, ProverService,
    StateTransitionInfo,
};

use super::{DefaultSpec, ProofStateRoot, ProofWitness};

type TestNetworkProverService = NetworkProverService<
    <DefaultSpec as Spec>::Address,
    ProofStateRoot,
    ProofWitness,
    MockDaService,
    SP1,
    MockZkvm,
>;

/// Tests proof generation using the SP1 network prover service.
///
/// This submits real proof requests to the Succinct proving network for per-block
/// (inner) proofs, and uses MockZkvmNetwork (auto-complete) for aggregation (outer).
///
/// Prerequisites:
///   - `NETWORK_PRIVATE_KEY` env var set (Succinct network auth)
///   - SP1 guest ELF built (`cargo build` in the prover guest directory)
///   - Network access to Succinct's proving infrastructure
#[tokio::test(flavor = "multi_thread")]
#[ignore = "Requires SP1_PRIVATE_KEY and network access to the Succinct proving network"]
async fn test_network_proof_generation() {
    tracing_subscriber::fmt::init();

    let elf: &[u8] = *sp1::SP1_GUEST_MOCK_ELF;
    assert!(
        !elf.is_empty(),
        "SP1 guest ELF is empty — build the guest first"
    );

    let inner_vm = SP1Network::new(elf)
        .await
        .expect("Failed to create SP1 network prover");
    // auto-complete outer proofs - real outer not supported yet
    let outer_vm = MockZkvmNetwork::new(true);

    let da_verifier = sov_mock_da::MockDaVerifier::default();
    let prover_address = <DefaultSpec as Spec>::Address::try_from([0u8; 28].as_ref()).unwrap();

    let prover_service = TestNetworkProverService::new(
        inner_vm,
        outer_vm,
        da_verifier,
        prover_address,
        Duration::from_secs(600),
    );

    let (genesis_state_root, witnesses) = super::generate_witnesses().await;

    // Submit all blocks to the network prover.
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
        tracing::info!("Block {} submitted to network prover", i);
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
                tracing::info!("Inner proofs still in progress, polling again in 30s...");
                tokio::time::sleep(Duration::from_secs(30)).await;
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
