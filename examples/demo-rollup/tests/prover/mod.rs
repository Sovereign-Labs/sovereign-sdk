use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use demo_stf::genesis_config::create_genesis_config;
use demo_stf::runtime::Runtime;
use sov_db::schema::SchemaBatch;
use sov_db::storage_manager::NativeStorageManager;
use sov_mock_da::{MockAddress, MockBlock, MockDaService, MockDaSpec};
use sov_modules_api::execution_mode::WitnessGeneration;
use sov_modules_api::{OperatingMode, SlotData};
use sov_modules_stf_blueprint::{GenesisParams, StfBlueprint};
use sov_rollup_interface::da::BlockHeaderTrait;
use sov_rollup_interface::node::da::DaService;
use sov_rollup_interface::stf::{ExecutionContext, StateTransitionFunction};
use sov_rollup_interface::storage::HierarchicalStorageManager;
use sov_rollup_interface::zk::StateTransitionWitness;
use sov_state::ProverStorage;
use sov_test_utils::generators::BlobBuildingCtx;
use sov_test_utils::TestStorageSpec;
use tempfile::TempDir;

use crate::prover::datagen::{get_blocks_from_da, DEFAULT_BLOCKS};
use crate::test_helpers::test_genesis_paths;

pub(super) type DefaultSpec = sov_modules_api::configurable_spec::ConfigurableSpec<
    sov_mock_da::MockDaSpec,
    sov_sp1_adapter::SP1,
    sov_mock_zkvm::MockZkvm,
    demo_stf::MultiAddressEvmSolana,
    WitnessGeneration,
    sov_mock_zkvm::MockZkvmCryptoSpec,
>;

mod datagen;
mod network;
mod parallel;
mod sp1_cpu_prover;

pub(super) type TestSTF = StfBlueprint<DefaultSpec, Runtime<DefaultSpec>>;
pub(super) type ProofStateRoot = <TestSTF as StateTransitionFunction<MockDaSpec>>::StateRoot;
pub(super) type ProofWitness = <TestSTF as StateTransitionFunction<MockDaSpec>>::Witness;
pub(super) type StfWitness = StateTransitionWitness<ProofStateRoot, ProofWitness, MockDaSpec>;

/// Executes the STF against mock DA blocks and produces per-block witnesses.
///
/// This is the shared data generation logic used by both the local (host) prover
/// tests and the network prover tests.
///
/// Returns `(genesis_state_root, witnesses)`.
pub(super) async fn generate_witnesses() -> (ProofStateRoot, Vec<StfWitness>) {
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
    // Write it to the database immediately!
    storage_manager.finalize(&genesis_block.header).unwrap();

    let genesis_state_root = prev_state_root;

    // TODO: Fix this with genesis logic.
    let mut blocks = get_blocks_from_da(sequencer_mode)
        .await
        .expect("Failed to get DA blocks");

    let mut witnesses = Vec::new();

    for filtered_block in &mut blocks[..(DEFAULT_BLOCKS as usize)] {
        let height = filtered_block.header().height();
        tracing::info!(
            "Requesting data for height {} and prev_state_root 0x{}",
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

        witnesses.push(StfWitness {
            initial_state_root: prev_state_root,
            da_block_header: filtered_block.header().clone(),
            relevant_proofs,
            witness: result.witness,
            relevant_blobs,
            final_state_root: result.state_root,
        });

        prev_state_root = result.state_root;
        storage_manager
            .save_change_set(
                filtered_block.header(),
                result.change_set,
                SchemaBatch::new(),
            )
            .unwrap();
    }

    (genesis_state_root, witnesses)
}
