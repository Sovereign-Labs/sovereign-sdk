use std::env;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[macro_use]
extern crate prettytable;

use anyhow::Context;
use demo_stf::genesis_config::{get_genesis_config, GenesisPaths};
use demo_stf::runtime::Runtime;
use prettytable::Table;
use prometheus::{Histogram, HistogramOpts, Registry};
use sov_db::ledger_db::{LedgerDB, SlotCommit};
use sov_modules_api::default_context::DefaultContext;
use sov_modules_stf_template::{AppTemplate, TxEffect};
use sov_risc0_adapter::host::Risc0Verifier;
use sov_rng_da_service::{RngDaService, RngDaSpec};
use sov_rollup_interface::mocks::{MockBlock, MockBlockHeader};
use sov_rollup_interface::services::da::DaService;
use sov_rollup_interface::stf::StateTransitionFunction;
use sov_rollup_interface::storage::StorageManager;
use sov_stf_runner::{from_toml_path, RollupConfig};
use tempfile::TempDir;

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

    let rollup_config_path = "benches/node/rollup_config.toml".to_string();
    let mut rollup_config: RollupConfig<sov_celestia_adapter::CelestiaConfig> =
        from_toml_path(&rollup_config_path)
            .context("Failed to read rollup configuration")
            .unwrap();

    let temp_dir = TempDir::new().expect("Unable to create temporary directory");
    rollup_config.storage.path = PathBuf::from(temp_dir.path());
    let ledger_db =
        LedgerDB::with_path(&rollup_config.storage.path).expect("Ledger DB failed to open");

    let da_service = Arc::new(RngDaService::new());

    let storage_config = sov_state::config::Config {
        path: rollup_config.storage.path,
    };
    let storage_manager = sov_state::storage_manager::ProverStorageManager::new(storage_config)
        .expect("ProverStorageManager initialization has failed");
    let stf = AppTemplate::<
        DefaultContext,
        RngDaSpec,
        Risc0Verifier,
        Runtime<DefaultContext, RngDaSpec>,
    >::new();

    let demo_genesis_config = get_genesis_config(&GenesisPaths::from_dir(
        "../test-data/genesis/integration-tests",
    ))
    .unwrap();

    let (mut current_root, _) =
        stf.init_chain(storage_manager.get_native_storage(), demo_genesis_config);

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

        let blob_txs = da_service.extract_relevant_blobs(&filtered_block);
        blobs.push(blob_txs);
    }

    // Setup. Block 0 has a single txn that creates the token. Exclude from timers
    let filtered_block = &blocks[0usize];
    let mut data_to_commit = SlotCommit::new(filtered_block.clone());
    let apply_block_results = stf.apply_slot(
        &current_root,
        storage_manager.get_native_storage(),
        Default::default(),
        &filtered_block.header,
        &filtered_block.validity_cond,
        &mut blobs[0usize],
    );
    current_root = apply_block_results.state_root;
    data_to_commit.add_batch(apply_block_results.batch_receipts[0].clone());

    ledger_db.commit_slot(data_to_commit).unwrap();

    // Rollup processing. Block 1 -> end are the transfer txns. Timers start here
    let total = Instant::now();
    let mut apply_block_time = Duration::new(0, 0);
    for height in start_height..=end_height {
        let filtered_block = &blocks[height as usize];

        let mut data_to_commit = SlotCommit::new(filtered_block.clone());

        let now = Instant::now();

        let apply_block_results = stf.apply_slot(
            &current_root,
            storage_manager.get_native_storage(),
            Default::default(),
            &filtered_block.header,
            &filtered_block.validity_cond,
            &mut blobs[height as usize],
        );
        current_root = apply_block_results.state_root;

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
