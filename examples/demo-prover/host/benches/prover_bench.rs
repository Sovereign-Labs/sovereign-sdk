use std::env;
use jupiter::types::FilteredCelestiaBlock;
use borsh::de::BorshDeserialize;
use std::fs::read_to_string;
use std::path::PathBuf;
use tempfile::TempDir;
use std::time::{Duration, Instant};

use anyhow::Context;
use const_rollup_config::{ROLLUP_NAMESPACE_RAW, SEQUENCER_DA_ADDRESS};
use demo_stf::app::{DefaultPrivateKey, NativeAppRunner};
use demo_stf::genesis_config::create_demo_genesis_config;
use demo_stf::runner_config::{from_toml_path, Config as RunnerConfig};
use jupiter::da_service::{CelestiaService, DaServiceConfig};
use jupiter::types::NamespaceId;
use jupiter::verifier::RollupParams;
use methods::{ROLLUP_ELF, ROLLUP_ID};
use risc0_adapter::host::Risc0Host;
use serde::Deserialize;
use sov_modules_api::RpcRunner;
use sov_rollup_interface::services::da::DaService;
use sov_rollup_interface::services::stf_runner::StateTransitionRunner;
use sov_rollup_interface::stf::StateTransitionFunction;
use sov_rollup_interface::zk::traits::ZkvmHost;
use sov_state::Storage;

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct RollupConfig {
    pub start_height: u64,
    pub da: DaServiceConfig,
    pub runner: RunnerConfig,
}

// The rollup stores its data in the namespace b"sov-test" on Celestia
const ROLLUP_NAMESPACE: NamespaceId = NamespaceId(ROLLUP_NAMESPACE_RAW);

fn main() -> Result<(), anyhow::Error> {
    let rollup_config_path = "benches/rollup_config.toml".to_string();
    let mut rollup_config: RollupConfig = from_toml_path(&rollup_config_path)
        .context("Failed to read rollup configuration")
        .unwrap();

    let temp_dir = TempDir::new().expect("Unable to create temporary directory");
    rollup_config.runner.storage.path = PathBuf::from(temp_dir.path());

    let da_service = CelestiaService::new(
        rollup_config.da.clone(),
        RollupParams {
            namespace: ROLLUP_NAMESPACE,
        },
    );

    let sequencer_private_key = DefaultPrivateKey::generate();

    let mut demo_runner = NativeAppRunner::<Risc0Host>::new(rollup_config.runner.clone());
    let demo = demo_runner.inner_mut();

    let genesis_config = create_demo_genesis_config(
        100000000,
        sequencer_private_key.default_address(),
        SEQUENCER_DA_ADDRESS.to_vec(),
        &sequencer_private_key,
        &sequencer_private_key,
    );
    println!("Starting from empty storage, initialization chain");
    demo.init_chain(genesis_config);
    demo.begin_slot(Default::default());

    let (prev_state_root, _) = demo.end_slot();
    let mut prev_state_root = prev_state_root.0;

    let hex_data = read_to_string("benches/blocks.hex").expect("Failed to read data");
    let borshed_blocks: Vec<FilteredCelestiaBlock> = hex_data
        .lines()
        .map(|line| {
            let bytes = hex::decode(line).expect("Failed to decode hex data");
            FilteredCelestiaBlock::try_from_slice(&bytes).expect("Failed to deserialize data")
        })
        .collect();

    for height in 0..(borshed_blocks.len() as u64) {
        let mut host = Risc0Host::new(ROLLUP_ELF);
        host.write_to_guest(prev_state_root);
        println!(
            "Requesting data for height {} and prev_state_root 0x{}",
            height,
            hex::encode(prev_state_root)
        );
        let filtered_block = &borshed_blocks[height as usize];
        let header_hash = hex::encode(filtered_block.header.header.hash());
        host.write_to_guest(&filtered_block.header);
        let (blob_txs, inclusion_proof, completeness_proof) =
            da_service.extract_relevant_txs_with_proof(&filtered_block);

        host.write_to_guest(&inclusion_proof);
        host.write_to_guest(&completeness_proof);

        demo.begin_slot(Default::default());
        if blob_txs.is_empty() {
            println!(
                "Block at height {} with header 0x{} has no batches, skip proving",
                height, header_hash
            );
            continue;
        }
        println!("Block has {} batches", blob_txs.len());
        for mut blob in blob_txs.clone() {
            let receipt = demo.apply_blob(&mut blob, None);
            println!(
                "batch with hash=0x{} has been applied",
                hex::encode(receipt.batch_hash)
            );
        }
        // Write txs only after they been read, so verification can be done properly
        host.write_to_guest(&blob_txs);

        let (next_state_root, witness) = demo.end_slot();
        host.write_to_guest(&witness);

        println!("Started proving block {height}");
        let now = Instant::now();
        let receipt = host.run().expect("Prover should run successfully");
        println!("prover time: {:?}",now.elapsed());
        println!("prover cycles: {}",host.cycles());
        println!("Start verifying..");
        receipt.verify(&ROLLUP_ID).expect("Receipt should be valid");

        prev_state_root = next_state_root.0;
        println!("Completed proving and verifying block {height}");
    }

    Ok(())
}
