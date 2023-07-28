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
use jupiter::BlobWithSender;
use methods::{ROLLUP_ELF, ROLLUP_ID};
use risc0_adapter::host::Risc0Host;
use serde::Deserialize;
use sov_modules_api::RpcRunner;
use sov_rollup_interface::services::da::DaService;
use sov_rollup_interface::services::stf_runner::StateTransitionRunner;
use sov_rollup_interface::stf::StateTransitionFunction;
use sov_rollup_interface::zk::ZkvmHost;
use sov_state::Storage;

#[cfg(feature = "bench")]
use risc0_adapter::host::GLOBAL_HASHMAP;

#[cfg(feature = "bench")]
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct RollupConfig {
    pub start_height: u64,
    pub da: DaServiceConfig,
    pub runner: RunnerConfig,
}

// The rollup stores its data in the namespace b"sov-test" on Celestia
const ROLLUP_NAMESPACE: NamespaceId = NamespaceId(ROLLUP_NAMESPACE_RAW);

#[macro_use]
extern crate prettytable;

use prettytable::Table;

fn print_cycle_averages(
    metric_map: HashMap<String, (u64,u64)>
) {

    let mut metrics_vec: Vec<(String, (u64,u64))> = metric_map.iter()
        .map(|(k, (sum, count))| (k.clone(), (((*sum as f64)/(*count as f64)).round() as u64, count.clone())))
        .collect();

    metrics_vec.sort_by(|a, b| b.1.cmp(&a.1));

    let mut table = Table::new();
    table.add_row(row!["Function", "Average Cycles", "Num Calls"]);
    for (k, (avg, count)) in metrics_vec {
        table.add_row(row![k, format!("{}", avg),  format!("{}",count)]);
    }
    table.printstd();

}

fn chain_stats(
    num_blocks: usize,
    num_blocks_with_txns : usize,
    num_txns: usize,
    num_blobs: usize
) {

    let mut table = Table::new();
    table.add_row(row!["Total blocks", num_blocks]);
    table.add_row(row!["Blocks with transactions", num_blocks_with_txns]);
    table.add_row(row!["Number of blobs", num_blobs]);
    table.add_row(row!["Total number of transactions", num_txns]);
    table.add_row(row!["Average number of transactions per block", ((num_txns as f64) / (num_blocks_with_txns as f64)) as u64]);
    table.printstd();

}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let rollup_config_path = "benches/rollup_config.toml".to_string();
    let mut rollup_config: RollupConfig = from_toml_path(&rollup_config_path)
        .context("Failed to read rollup configuration")
        .unwrap();

    let mut num_blocks = 0;
    let mut num_blobs = 0;
    let mut num_blocks_with_txns = 0;
    let mut num_total_transactions = 0;

    let temp_dir = TempDir::new().expect("Unable to create temporary directory");
    rollup_config.runner.storage.path = PathBuf::from(temp_dir.path());

    let da_service = CelestiaService::new(
        rollup_config.da.clone(),
        RollupParams {
            namespace: ROLLUP_NAMESPACE,
        },
    ).await;

    let sequencer_private_key = DefaultPrivateKey::generate();

    let mut demo_runner = NativeAppRunner::<Risc0Host,BlobWithSender>::new(rollup_config.runner.clone());
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
        num_blocks+=1;
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
            da_service.extract_relevant_txs_with_proof(&filtered_block).await;

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
        num_blocks_with_txns+=1;
        println!("Block has {} batches", blob_txs.len());
        for mut blob in blob_txs.clone() {
            num_blobs+=1;
            let receipt = demo.apply_blob(&mut blob, None);
            num_total_transactions += receipt.tx_receipts.len();
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
        println!("Skipping prover to capture cycle counts\n");
        let receipt = host.run_without_proving().expect("Prover should run successfully");
        println!("prover time: {:?}\n\n",now.elapsed());
        println!("==================================================\n");
        prev_state_root = next_state_root.0;

    }

    #[cfg(feature = "bench")]
    {
        let hashmap_guard = GLOBAL_HASHMAP.lock();
        let metric_map = hashmap_guard.clone();
        let total_cycles = metric_map.get("Cycles per block").unwrap().0;
        println!("\nBlock stats\n");
        chain_stats(num_blocks, num_blocks_with_txns, num_total_transactions, num_blobs);
        println!("\nCycle Metrics\n");
        print_cycle_averages(metric_map);
        println!("\nTotal cycles consumed for test: {}\n", total_cycles);

    }

    Ok(())
}
