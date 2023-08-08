use std::env;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context;
use const_rollup_config::SEQUENCER_DA_ADDRESS;
use demo_stf::app::App;
use demo_stf::genesis_config::create_demo_genesis_config;
use jupiter::verifier::address::CelestiaAddress;
use prometheus::{Histogram, HistogramOpts, Registry};
use risc0_adapter::host::Risc0Verifier;
use sov_db::ledger_db::{LedgerDB, SlotCommit};
use sov_demo_rollup::rng_xfers::RngDaService;
use sov_modules_api::default_signature::private_key::DefaultPrivateKey;
use sov_modules_api::PrivateKey;
use sov_rollup_interface::mocks::{TestBlob, TestBlock, TestBlockHeader, TestHash};
use sov_rollup_interface::services::da::DaService;
use sov_rollup_interface::stf::StateTransitionFunction;
use sov_stf_runner::{from_toml_path, RollupConfig};
use tempfile::TempDir;

#[macro_use]
extern crate prettytable;

use prettytable::Table;

fn print_times(total: Duration, apply_block_time: Duration, blocks: u64, num_txns: u64) {
    let mut table = Table::new();

    table.add_row(row!["Blocks", format!("{:?}", blocks)]);
    table.add_row(row!["Txns per Block", format!("{:?}", num_txns)]);
    table.add_row(row!["Total", format!("{:?}", total)]);
    table.add_row(row!["Apply Block", format!("{:?}", apply_block_time)]);
    table.add_row(row![
        "Txns per sec (TPS)",
        format!(
            "{:?}",
            ((blocks * num_txns) as f64) / (total.as_secs() as f64)
        )
    ]);

    // Print the table to stdout
    table.printstd();
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let registry = Registry::new();
    let h_apply_block = Histogram::with_opts(HistogramOpts::new(
        "block_processing_apply_block",
        "Histogram of block processing - apply blob times",
    ))
    .expect("Failed to create histogram");

    registry
        .register(Box::new(h_apply_block.clone()))
        .expect("Failed to register apply blob histogram");

    let start_height: u64 = 0u64;
    let mut end_height: u64 = 10u64;
    let mut num_txns = 10000;
    let mut timer_output = true;
    let mut prometheus_output = false;
    if let Ok(val) = env::var("TXNS_PER_BLOCK") {
        num_txns = val
            .parse()
            .expect("TXNS_PER_BLOCK var should be a +ve number");
    }
    if let Ok(val) = env::var("BLOCKS") {
        end_height = val.parse().expect("BLOCKS var should be a +ve number");
    }
    if let Ok(_val) = env::var("PROMETHEUS_OUTPUT") {
        prometheus_output = true;
        timer_output = false;
    }
    if let Ok(_val) = env::var("TIMER_OUTPUT") {
        timer_output = true;
    }

    let rollup_config_path = "benches/rollup_config.toml".to_string();
    let mut rollup_config: RollupConfig = from_toml_path(&rollup_config_path)
        .context("Failed to read rollup configuration")
        .unwrap();

    let temp_dir = TempDir::new().expect("Unable to create temporary directory");
    rollup_config.runner.storage.path = PathBuf::from(temp_dir.path());
    let ledger_db =
        LedgerDB::with_path(&rollup_config.runner.storage.path).expect("Ledger DB failed to open");

    let da_service = Arc::new(RngDaService::new());

    let demo_runner =
        App::<Risc0Verifier, TestBlob<CelestiaAddress>>::new(rollup_config.runner.storage);

    let mut demo = demo_runner.stf;
    let sequencer_private_key = DefaultPrivateKey::generate();
    let demo_genesis_config = create_demo_genesis_config(
        100000000,
        sequencer_private_key.default_address(),
        SEQUENCER_DA_ADDRESS.to_vec(),
        &sequencer_private_key,
        &sequencer_private_key,
    );
    let _prev_state_root = {
        // Check if the rollup has previously been initialized
        demo.init_chain(demo_genesis_config);
        let apply_block_result = demo.apply_slot(Default::default(), []);
        let prev_state_root = apply_block_result.state_root;
        prev_state_root.0
    };

    // data generation
    let mut blobs = vec![];
    let mut blocks = vec![];

    for height in start_height..end_height {
        let num_bytes = height.to_le_bytes();
        let mut barray = [0u8; 32];
        barray[..num_bytes.len()].copy_from_slice(&num_bytes);
        let filtered_block = TestBlock {
            curr_hash: barray,
            header: TestBlockHeader {
                prev_hash: TestHash([0u8; 32]),
            },
            height,
        };
        blocks.push(filtered_block.clone());

        let blob_txs = da_service.extract_relevant_txs(&filtered_block);
        blobs.push(blob_txs);
    }

    // rollup processing
    let total = Instant::now();
    let mut apply_block_time = Duration::new(0, 0);
    for height in start_height..end_height {
        let filtered_block = &blocks[height as usize];

        let mut data_to_commit = SlotCommit::new(filtered_block.clone());

        let now = Instant::now();

        let apply_block_results = demo.apply_slot(Default::default(), &mut blobs[height as usize]);

        apply_block_time += now.elapsed();
        h_apply_block.observe(now.elapsed().as_secs_f64());

        for receipt in apply_block_results.batch_receipts {
            data_to_commit.add_batch(receipt);
        }

        ledger_db.commit_slot(data_to_commit).unwrap();
    }

    let total = total.elapsed();
    if timer_output {
        print_times(total, apply_block_time, end_height, num_txns);
    }
    if prometheus_output {
        println!("{:#?}", registry.gather());
    }
    Ok(())
}
