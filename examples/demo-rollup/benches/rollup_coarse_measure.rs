mod rng_xfers;
use std::env;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context;
use demo_stf::app::App;
use demo_stf::genesis_config::create_demo_genesis_config;
use prometheus::{Histogram, HistogramOpts, Registry};
use rng_xfers::{RngDaService, RngDaSpec, SEQUENCER_DA_ADDRESS};
use sov_db::ledger_db::{LedgerDB, SlotCommit};
use sov_modules_api::default_signature::private_key::DefaultPrivateKey;
use sov_modules_api::PrivateKey;
use sov_risc0_adapter::host::Risc0Verifier;
use sov_rollup_interface::mocks::{MockAddress, MockBlock, MockBlockHeader};
use sov_rollup_interface::services::da::DaService;
use sov_rollup_interface::stf::StateTransitionFunction;
use sov_stf_runner::{from_toml_path, RollupConfig};
use tempfile::TempDir;

#[macro_use]
extern crate prettytable;

use prettytable::Table;
use sov_modules_stf_template::TxEffect;

fn print_times(
    total: Duration,
    apply_block_time: Duration,
    blocks: u64,
    num_txns: u64,
    num_success_txns: u64,
) {
    let mut table = Table::new();

    table.add_row(row!["Blocks", format!("{:?}", blocks)]);
    table.add_row(row!["Txns per Block", format!("{:?}", num_txns)]);
    table.add_row(row![
        "Processed Txns (Success)",
        format!("{:?}", num_success_txns)
    ]);
    table.add_row(row!["Total", format!("{:?}", total)]);
    table.add_row(row!["Apply Block", format!("{:?}", apply_block_time)]);
    table.add_row(row![
        "Txns per sec (TPS)",
        format!("{:?}", ((blocks * num_txns) as f64) / total.as_secs_f64())
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

    let start_height: u64 = 1u64;
    let mut end_height: u64 = 10u64;
    let mut num_success_txns = 0u64;
    let mut num_txns_per_block = 10000;
    let mut timer_output = true;
    let mut prometheus_output = false;
    if let Ok(val) = env::var("TXNS_PER_BLOCK") {
        num_txns_per_block = val
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
    let mut rollup_config: RollupConfig<sov_celestia_adapter::DaServiceConfig> =
        from_toml_path(&rollup_config_path)
            .context("Failed to read rollup configuration")
            .unwrap();

    let temp_dir = TempDir::new().expect("Unable to create temporary directory");
    rollup_config.storage.path = PathBuf::from(temp_dir.path());
    let ledger_db =
        LedgerDB::with_path(&rollup_config.storage.path).expect("Ledger DB failed to open");

    let da_service = Arc::new(RngDaService::new());

    let demo_runner = App::<Risc0Verifier, RngDaSpec>::new(rollup_config.storage);

    let mut demo = demo_runner.stf;
    let sequencer_private_key = DefaultPrivateKey::generate();
    let sequencer_da_address = MockAddress::from(SEQUENCER_DA_ADDRESS);
    let demo_genesis_config = create_demo_genesis_config(
        100000000,
        sequencer_private_key.default_address(),
        sequencer_da_address.as_ref().to_vec(),
        &sequencer_private_key,
        #[cfg(feature = "experimental")]
        Default::default(),
    );

    demo.init_chain(demo_genesis_config);

    // data generation
    let mut blobs = vec![];
    let mut blocks = vec![];
    for height in 0..=end_height {
        let num_bytes = height.to_le_bytes();
        let mut barray = [0u8; 32];
        barray[..num_bytes.len()].copy_from_slice(&num_bytes);
        let filtered_block = MockBlock {
            header: MockBlockHeader {
                prev_hash: [0u8; 32].into(),
                hash: barray.into(),
                height,
            },
            validity_cond: Default::default(),
            blobs: Default::default(),
        };
        blocks.push(filtered_block.clone());

        let blob_txs = da_service.extract_relevant_txs(&filtered_block);
        blobs.push(blob_txs);
    }

    // Setup. Block 0 has a single txn that creates the token. Exclude from timers
    let filtered_block = &blocks[0usize];
    let mut data_to_commit = SlotCommit::new(filtered_block.clone());
    let apply_block_results = demo.apply_slot(
        Default::default(),
        &filtered_block.header,
        &filtered_block.validity_cond,
        &mut blobs[0usize],
    );
    data_to_commit.add_batch(apply_block_results.batch_receipts[0].clone());

    ledger_db.commit_slot(data_to_commit).unwrap();

    // Rollup processing. Block 1 -> end are the transfer txns. Timers start here
    let total = Instant::now();
    let mut apply_block_time = Duration::new(0, 0);
    for height in start_height..=end_height {
        let filtered_block = &blocks[height as usize];

        let mut data_to_commit = SlotCommit::new(filtered_block.clone());

        let now = Instant::now();

        let apply_block_results = demo.apply_slot(
            Default::default(),
            &filtered_block.header,
            &filtered_block.validity_cond,
            &mut blobs[height as usize],
        );

        apply_block_time += now.elapsed();
        h_apply_block.observe(now.elapsed().as_secs_f64());
        for receipt in apply_block_results.batch_receipts {
            for t in &receipt.tx_receipts {
                if t.receipt == TxEffect::Successful {
                    num_success_txns += 1
                }
            }
            data_to_commit.add_batch(receipt);
        }

        ledger_db.commit_slot(data_to_commit).unwrap();
    }

    let total = total.elapsed();
    if timer_output {
        print_times(
            total,
            apply_block_time,
            end_height,
            num_txns_per_block,
            num_success_txns,
        );
    }
    if prometheus_output {
        println!("{:#?}", registry.gather());
    }
    Ok(())
}
