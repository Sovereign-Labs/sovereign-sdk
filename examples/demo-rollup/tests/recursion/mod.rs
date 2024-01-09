// use risc0_zkvm::ProverOpts;
// use risc0_zkvm::{
//     get_prover_server, ExecutorEnv, ExecutorEnvBuilder, ExecutorImpl, InnerReceipt, Journal,
//     Receipt, Session,
// };
// use sov_modules_api::da::BlockHeaderTrait;
// use sov_modules_api::SlotData;
// use std::{
//     env,
//     path::{Path, PathBuf},
// };

// mod datagen;

// use anyhow::{Context, Result};
// use demo_stf::genesis_config::{get_genesis_config, GenesisPaths};
// use demo_stf::runtime::Runtime;
// use risc0::MOCK_DA_ELF;
// use sov_mock_da::{MockAddress, MockBlock, MockDaConfig, MockDaService, MockDaSpec};
// use sov_modules_api::default_context::DefaultContext;
// use sov_modules_stf_blueprint::{
//     kernels::basic::{BasicKernel, BasicKernelGenesisConfig},
//     GenesisParams, StfBlueprint,
// };
// use sov_prover_storage_manager::ProverStorageManager;
// use sov_risc0_adapter::host::Risc0Host;
// use sov_rollup_interface::{
//     services::da::DaService,
//     stf::StateTransitionFunction,
//     storage::HierarchicalStorageManager,
//     zk::{StateTransitionData, ZkvmHost},
// };
// use sov_state::DefaultStorageSpec;
// use sov_stf_runner::{from_toml_path, read_json_file, RollupConfig};
// use tempfile::TempDir;

// use crate::recursion::datagen::{generate_genesis_config, get_bench_blocks};

// const DEFAULT_GENESIS_CONFIG_DIR: &str = "../test-data/genesis/benchmark";

// type BenchSTF<'a> = StfBlueprint<
//     DefaultContext,
//     MockDaSpec,
//     Risc0Host<'a>,
//     Runtime<DefaultContext, MockDaSpec>,
//     BasicKernel<DefaultContext, MockDaSpec>,
// >;

// fn config_and_init_rollup<'a>() -> Result<
//     (
//         <BenchSTF<'a> as StateTransitionFunction<Risc0Host<'a>, MockDaSpec>>::StateRoot,
//         ProverStorageManager<MockDaSpec, DefaultStorageSpec>,
//         BenchSTF<'a>,
//         MockDaService,
//     ),
//     anyhow::Error,
// > {
//     let genesis_conf_dir = match env::var("GENESIS_CONFIG_DIR") {
//         Ok(dir) => dir,
//         Err(_) => {
//             println!("GENESIS_CONFIG_DIR not set, using default");
//             String::from(DEFAULT_GENESIS_CONFIG_DIR)
//         }
//     };

//     let rollup_config_path = "tests/recursion/rollup_config.toml".to_string();
//     let mut rollup_config: RollupConfig<MockDaConfig> = from_toml_path(rollup_config_path)
//         .context("Failed to read rollup configuration")
//         .unwrap();

//     let temp_dir = TempDir::new().expect("Unable to create temporary directory");
//     rollup_config.storage.path = PathBuf::from(temp_dir.path());
//     let da_service = MockDaService::new(MockAddress::default());
//     let storage_config = sov_state::config::Config {
//         path: rollup_config.storage.path,
//     };

//     let mut storage_manager =
//         ProverStorageManager::<MockDaSpec, DefaultStorageSpec>::new(storage_config)
//             .expect("ProverStorageManager initialization has failed");
//     let stf = BenchSTF::new();

//     generate_genesis_config(genesis_conf_dir.as_str())?;

//     let genesis_config = {
//         let rt_params = get_genesis_config::<DefaultContext, _>(&GenesisPaths::from_dir(
//             genesis_conf_dir.as_str(),
//         ))
//         .unwrap();

//         let chain_state =
//             read_json_file(Path::new(genesis_conf_dir.as_str()).join("chain_state.json")).unwrap();
//         let kernel_params = BasicKernelGenesisConfig { chain_state };
//         GenesisParams {
//             runtime: rt_params,
//             kernel: kernel_params,
//         }
//     };

//     println!("Starting from empty storage, initialization chain");
//     let genesis_block = MockBlock::default();
//     let (mut prev_state_root, storage) = stf.init_chain(
//         storage_manager
//             .create_storage_on(genesis_block.header())
//             .unwrap(),
//         genesis_config,
//     );
//     storage_manager
//         .save_change_set(genesis_block.header(), storage)
//         .unwrap();
//     // Write it to the database immediately!
//     storage_manager.finalize(&genesis_block.header).unwrap();

//     Ok((prev_state_root, storage_manager, stf, da_service))
// }

// #[tokio::main]
// async fn main() -> Result<(), anyhow::Error> {
//     let (mut prev_state_root, mut storage_manager, stf, da_service) = config_and_init_rollup()?;

//     let blocks = get_bench_blocks().await?;

//     let mut num_blocks = 0;
//     let mut num_blobs = 0;
//     let mut num_blocks_with_txns = 0;
//     let mut num_total_transactions = 0;

//     for filtered_block in &blocks {
//         num_blocks += 1;
//         let mut host = Risc0Host::new(MOCK_DA_ELF);

//         let height = filtered_block.header().height();
//         println!(
//             "Requesting data for height {} and prev_state_root 0x{}",
//             height,
//             hex::encode(prev_state_root.0)
//         );
//         let (mut blob_txs, inclusion_proof, completeness_proof) = da_service
//             .extract_relevant_blobs_with_proof(filtered_block)
//             .await;

//         if !blob_txs.is_empty() {
//             num_blobs += blob_txs.len();
//         }

//         let storage = storage_manager
//             .create_storage_on(filtered_block.header())
//             .unwrap();

//         let result = stf.apply_slot(
//             &prev_state_root,
//             storage,
//             Default::default(),
//             filtered_block.header(),
//             &filtered_block.validity_condition(),
//             &mut blob_txs,
//         );

//         for r in result.batch_receipts {
//             let num_tx = r.tx_receipts.len();
//             num_total_transactions += num_tx;
//             if num_tx > 0 {
//                 num_blocks_with_txns += 1;
//             }
//         }

//         let data = StateTransitionData::<
//             <BenchSTF as StateTransitionFunction<Risc0Host<'_>, MockDaSpec>>::StateRoot,
//             <BenchSTF as StateTransitionFunction<Risc0Host<'_>, MockDaSpec>>::Witness,
//             MockDaSpec,
//         > {
//             initial_state_root: prev_state_root,
//             da_block_header: filtered_block.header().clone(),
//             inclusion_proof,
//             completeness_proof,
//             state_transition_witness: result.witness,
//             blobs: blob_txs,
//             final_state_root: result.state_root,
//         };
//         host.add_hint(data);

//         println!("Running zkVM proof");
//         let segment_limit_po2 = 16; // 64k cycles
//         let cycles = 1 << segment_limit_po2;
//         let env = ExecutorEnv::builder()
//             .write(&MultiTestSpec::BusyLoop { cycles })
//             .unwrap()
//             .segment_limit_po2(segment_limit_po2)
//             .build()
//             .unwrap();

//         tracing::info!("Executing rv32im");
//         let mut exec = ExecutorImpl::from_elf(env, MULTI_TEST_ELF).unwrap();
//         let session = exec.run().unwrap();
//         let segments = session.resolve().unwrap();
//         tracing::info!("Got {} segments", segments.len());

//         let opts = ProverOpts {
//             hashfn: hashfn.to_string(),
//         };

//         let prover = get_prover_server(&opts).unwrap();

//         let session = host.run_without_proving().unwrap();
//         let receipts = session.prove().unwrap();

//         println!("==================================================\n");
//         prev_state_root = result.state_root;
//         storage_manager
//             .save_change_set(filtered_block.header(), result.change_set)
//             .unwrap();
//         // TODO: Do we want to finalize some older blocks
//     }

//     Ok(())
// }

// #[test]
// fn recursion_risc0() {}
