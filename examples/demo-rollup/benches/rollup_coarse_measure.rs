use std::env;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context;
use const_rollup_config::SEQUENCER_DA_ADDRESS;
use demo_stf::app::NativeAppRunner;
use demo_stf::genesis_config::create_demo_genesis_config;
use demo_stf::runner_config::from_toml_path;
use prometheus::{Histogram, HistogramOpts, Registry};
use risc0_adapter::host::Risc0Verifier;
use sov_db::ledger_db::{LedgerDB, SlotCommit};
use sov_demo_rollup::config::RollupConfig;
use sov_demo_rollup::rng_xfers::RngDaService;
use sov_modules_api::default_signature::private_key::DefaultPrivateKey;
use sov_rollup_interface::mocks::{TestBlock, TestBlockHeader, TestHash};
use sov_rollup_interface::services::da::DaService;
use sov_rollup_interface::services::stf_runner::StateTransitionRunner;
use sov_rollup_interface::stf::StateTransitionFunction;
use tempfile::TempDir;

#[macro_use]
extern crate prettytable;
use prettytable::Table;

fn print_times(
    total: Duration,
    begin_slot_time: Duration,
    end_slot_time: Duration,
    apply_blob_time: Duration,
    blocks: u64,
    num_txns: u64,
) {
    let mut table = Table::new();

    table.add_row(row!["Blocks", format!("{:?}", blocks)]);
    table.add_row(row!["Txns per Block", format!("{:?}", num_txns)]);
    table.add_row(row!["Total", format!("{:?}", total)]);
    table.add_row(row!["Begin slot", format!("{:?}", begin_slot_time)]);
    table.add_row(row!["End slot", format!("{:?}", end_slot_time)]);
    table.add_row(row!["Apply Blob", format!("{:?}", apply_blob_time)]);
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
    let h_apply_blob = Histogram::with_opts(HistogramOpts::new(
        "block_processing_apply_blob",
        "Histogram of block processing - apply blob times",
    ))
    .expect("Failed to create histogram");
    let h_begin_slot = Histogram::with_opts(HistogramOpts::new(
        "block_processing_begin_slot",
        "Histogram of block processing - begin slot times",
    ))
    .expect("Failed to create histogram");
    let h_end_slot = Histogram::with_opts(HistogramOpts::new(
        "block_processing_end_slot",
        "Histogram of block processing - end slot times",
    ))
    .expect("Failed to create histogram");
    registry
        .register(Box::new(h_apply_blob.clone()))
        .expect("Failed to register apply blob histogram");
    registry
        .register(Box::new(h_begin_slot.clone()))
        .expect("Failed to register begin slot histogram");
    registry
        .register(Box::new(h_end_slot.clone()))
        .expect("Failed to register end slot histogram");

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

    let mut demo_runner = NativeAppRunner::<Risc0Verifier>::new(rollup_config.runner);

    let demo = demo_runner.inner_mut();
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
        demo.begin_slot(Default::default());
        let (prev_state_root, _) = demo.end_slot();
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
        blobs.push(blob_txs.clone());
    }

    // rollup processing
    let total = std::time::Instant::now();
    let mut begin_slot_time = Duration::new(0, 0);
    let mut end_slot_time = Duration::new(0, 0);
    let mut apply_blob_time = Duration::new(0, 0);
    for height in start_height..end_height {
        let filtered_block = &blocks[height as usize];

        let mut data_to_commit = SlotCommit::new(filtered_block.clone());

        let now = Instant::now();
        demo.begin_slot(Default::default());
        begin_slot_time += now.elapsed();
        h_begin_slot.observe(now.elapsed().as_secs_f64());

        for blob in &mut blobs[height as usize] {
            let now = Instant::now();
            let receipts = demo.apply_blob(blob, None);
            apply_blob_time += now.elapsed();
            h_apply_blob.observe(now.elapsed().as_secs_f64());
            data_to_commit.add_batch(receipts);
        }

        let now = Instant::now();
        let (_next_state_root, _witness) = demo.end_slot();
        end_slot_time += now.elapsed();
        h_end_slot.observe(now.elapsed().as_secs_f64());

        ledger_db.commit_slot(data_to_commit).unwrap();
    }

    let total = total.elapsed();
    if timer_output {
        print_times(
            total,
            begin_slot_time,
            end_slot_time,
            apply_blob_time,
            end_height,
            num_txns,
        );
    }
    if prometheus_output {
        println!("{:#?}", registry.gather());
    }
    Ok(())
}
