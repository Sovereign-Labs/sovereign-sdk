use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::time::Duration;

use demo_stf::genesis_config::create_genesis_config;
use demo_stf::runtime::Runtime;
use sov_db::schema::SchemaBatch;
use sov_db::storage_manager::NativeStorageManager;
use sov_mock_da::{MockAddress, MockBlock, MockDaService, MockDaSpec};
use sov_mock_zkvm::{MockZkvm, MockZkvmNetwork};
use sov_modules_api::execution_mode::WitnessGeneration;
use sov_modules_api::{OperatingMode, SlotData, Spec};
use sov_modules_stf_blueprint::{GenesisParams, StfBlueprint};
use sov_rollup_interface::common::SlotNumber;
use sov_rollup_interface::da::BlockHeaderTrait;
use sov_rollup_interface::node::da::DaService;
use sov_rollup_interface::stf::{ExecutionContext, StateTransitionFunction};
use sov_rollup_interface::storage::HierarchicalStorageManager;
use sov_rollup_interface::zk::StateTransitionWitness;
use sov_sp1_adapter::network::SP1Network;
use sov_sp1_adapter::SP1;
use sov_state::ProverStorage;
use sov_stf_runner::processes::{
    NetworkProverService, ProofAggregationStatus, ProofProcessingStatus, ProverService,
    StateTransitionInfo,
};
use sov_test_utils::generators::BlobBuildingCtx;
use sov_test_utils::TestStorageSpec;
use tempfile::TempDir;

use crate::prover::datagen::{get_blocks_from_da, DEFAULT_BLOCKS};
use crate::test_helpers::test_genesis_paths;

type DefaultSpec = sov_modules_api::configurable_spec::ConfigurableSpec<
    sov_mock_da::MockDaSpec,
    sov_sp1_adapter::SP1,
    sov_mock_zkvm::MockZkvm,
    demo_stf::MultiAddressEvmSolana,
    WitnessGeneration,
    sov_mock_zkvm::MockZkvmCryptoSpec,
>;

type TestSTF = StfBlueprint<DefaultSpec, Runtime<DefaultSpec>>;
type ProofStateRoot = <TestSTF as StateTransitionFunction<SP1, MockZkvm, MockDaSpec>>::StateRoot;
type ProofWitness = <TestSTF as StateTransitionFunction<SP1, MockZkvm, MockDaSpec>>::Witness;

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
///   - `SP1_PRIVATE_KEY` env var set (Succinct network auth)
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
        .expect("Failed to create SP1Network — is SP1_PRIVATE_KEY set?");
    let outer_vm = MockZkvmNetwork::new(true); // auto-complete outer proofs

    let da_verifier = sov_mock_da::MockDaVerifier::default();
    let prover_address = <DefaultSpec as Spec>::Address::try_from([0u8; 28].as_ref()).unwrap();

    let prover_service = TestNetworkProverService::new(
        inner_vm,
        outer_vm,
        da_verifier,
        Default::default(),
        prover_address,
        Duration::from_secs(600),
    );

    let (block_hashes, genesis_state_root) =
        generate_witnesses_and_prove(&prover_service).await;

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

/// Generates witnesses by executing the STF against mock DA blocks, then submits
/// each block to the network prover service.
///
/// Returns the block header hashes and the genesis state root (needed for aggregation).
async fn generate_witnesses_and_prove(
    prover_service: &TestNetworkProverService,
) -> (
    Vec<<MockDaSpec as sov_rollup_interface::da::DaSpec>::SlotHash>,
    ProofStateRoot,
) {
    let temp_dir = TempDir::new().expect("Unable to create temporary directory");
    tracing::info!("Creating temp dir at {}", temp_dir.path().display());

    let da_service = MockDaService::new(MockAddress::default());
    let sequencer_mode = BlobBuildingCtx::Preferred {
        curr_sequence_number: Arc::new(AtomicU64::new(0)),
    };

    let mut storage_manager =
        NativeStorageManager::<MockDaSpec, ProverStorage<TestStorageSpec>>::new(temp_dir.path())
            .expect("NativeStorageManager initialization has failed");
    let stf = TestSTF::new();

    let genesis_config = {
        let rt_params =
            create_genesis_config::<DefaultSpec>(&test_genesis_paths(OperatingMode::Zk)).unwrap();
        GenesisParams { runtime: rt_params }
    };

    tracing::info!("Starting from empty storage, initialization chain");
    let genesis_block = MockBlock::default();
    let (stf_state, _) = storage_manager
        .create_state_for(genesis_block.header())
        .unwrap();
    let (mut prev_state_root, stf_state) =
        stf.init_chain(&Default::default(), stf_state, genesis_config);
    storage_manager
        .save_change_set(genesis_block.header(), stf_state, SchemaBatch::new())
        .unwrap();
    storage_manager.finalize(&genesis_block.header).unwrap();

    let genesis_state_root = prev_state_root;

    let mut blocks = get_blocks_from_da(sequencer_mode)
        .await
        .expect("Failed to get DA blocks");

    let mut block_hashes = Vec::new();

    for (i, filtered_block) in blocks[..(DEFAULT_BLOCKS as usize)].iter_mut().enumerate() {
        let height = filtered_block.header().height();
        tracing::info!(
            "Processing block {} (height {}) with prev_state_root 0x{}",
            i,
            height,
            hex::encode(prev_state_root.root_hash())
        );

        let (mut relevant_blobs, relevant_proofs) = da_service
            .extract_relevant_blobs_with_proof(filtered_block)
            .await;

        let (stf_state, _) = storage_manager
            .create_state_for(filtered_block.header())
            .unwrap();

        let result = stf.apply_slot(
            &prev_state_root,
            stf_state,
            Default::default(),
            filtered_block.header(),
            relevant_blobs.as_iters(),
            ExecutionContext::Node,
        );

        let block_header_hash = filtered_block.header().hash();
        block_hashes.push(block_header_hash);

        let witness = StateTransitionWitness {
            initial_state_root: prev_state_root,
            da_block_header: filtered_block.header().clone(),
            relevant_proofs,
            witness: result.witness,
            relevant_blobs,
            final_state_root: result.state_root,
        };

        let slot_number = SlotNumber::new(i as u64 + 1);
        let state_transition_info = StateTransitionInfo::new(witness, slot_number);

        let status = prover_service
            .prove(state_transition_info)
            .await
            .expect("prove() should succeed");
        assert!(
            matches!(status, ProofProcessingStatus::ProvingInProgress),
            "Expected ProvingInProgress after submitting block {i}"
        );
        tracing::info!("Block {} submitted to network prover", i);

        prev_state_root = result.state_root;
        storage_manager
            .save_change_set(
                filtered_block.header(),
                result.change_set,
                SchemaBatch::new(),
            )
            .unwrap();
    }

    tracing::info!(
        "All {} blocks submitted, waiting for proofs...",
        block_hashes.len()
    );

    (block_hashes, genesis_state_root)
}
