#[macro_use]
extern crate prettytable;

use std::env;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context;
use demo_stf::genesis_config::{get_genesis_config, GenesisPaths};
use demo_stf::runtime::Runtime;
use humantime::format_duration;
use prettytable::Table;
use prometheus::{Histogram, HistogramOpts, Registry};
use sov_db::ledger_db::{LedgerDB, SlotCommit};
use sov_mock_da::{MockBlock, MockBlockHeader, MockDaSpec};
use sov_modules_api::default_context::DefaultContext;
use sov_modules_stf_blueprint::kernels::basic::{BasicKernel, BasicKernelGenesisConfig};
use sov_modules_stf_blueprint::{GenesisParams, StfBlueprint, TxEffect};
use sov_prover_storage_manager::ProverStorageManager;
use sov_risc0_adapter::host::Risc0Verifier;
use sov_rng_da_service::{RngDaService, RngDaSpec};
use sov_rollup_interface::da::BlockHeaderTrait;
use sov_rollup_interface::services::da::{DaService, SlotData};
use sov_rollup_interface::stf::StateTransitionFunction;
use sov_rollup_interface::storage::HierarchicalStorageManager;
use sov_state::DefaultStorageSpec;
use sov_stf_runner::{from_toml_path, read_json_file, RollupConfig};
use tempfile::TempDir;

fn print_times(
    total: Duration,
    apply_block_time: Duration,
    blocks: u64,
    num_txns: u64,
    num_success_txns: u64,
) {
    let mut table = Table::new();

    let total_txns = blocks * num_txns;
    table.add_row(row!["Blocks", format!("{:?}", blocks)]);
    table.add_row(row!["Transactions per block", format!("{:?}", num_txns)]);
    table.add_row(row![
        "Processed transactions (success/total)",
        format!("{:?}/{:?}", num_success_txns, total_txns)
    ]);
    table.add_row(row!["Total", format_duration(total)]);
    table.add_row(row!["Apply block", format_duration(apply_block_time)]);
    let tps = (total_txns as f64) / total.as_secs_f64();
    table.add_row(row!["Transactions per sec (TPS)", format!("{:.1}", tps)]);

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

    let mut end_height: u64 = 10;
    let mut num_success_txns = 0;
    let mut num_txns_per_block = 10000;
    let mut timer_output = true;
    let mut prometheus_output = false;
    if let Ok(val) = env::var("TXNS_PER_BLOCK") {
        num_txns_per_block = val
            .parse()
            .expect("TXNS_PER_BLOCK var should be a +ve number");
    }
    if let Ok(val) = env::var("BLOCKS") {
        end_height = val
            .parse::<u64>()
            .expect("BLOCKS var should be a +ve number")
            + 1;
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
        from_toml_path(rollup_config_path)
            .context("Failed to read rollup configuration")
            .unwrap();

    let temp_dir = TempDir::new().expect("Unable to create temporary directory");
    rollup_config.storage.path = PathBuf::from(temp_dir.path());
    let ledger_db =
        LedgerDB::with_path(&rollup_config.storage.path).expect("Ledger DB failed to open");

    let da_service = Arc::new(RngDaService::new());

    let storage_config = sov_state::config::Config {
        path: rollup_config.storage.path.clone(),
    };
    let mut storage_manager =
        ProverStorageManager::<MockDaSpec, DefaultStorageSpec>::new(storage_config)
            .expect("ProverStorage initialization failed");

    let genesis_block_header = MockBlockHeader::from_height(0);

    let storage = storage_manager
        .create_storage_on(&genesis_block_header)
        .expect("Getting genesis storage failed");

    let stf = StfBlueprint::<
        DefaultContext,
        RngDaSpec,
        Risc0Verifier,
        Runtime<DefaultContext, RngDaSpec>,
        BasicKernel<DefaultContext, _>,
    >::new();

    let demo_genesis_config = {
        let integ_test_conf_dir: &Path = "../test-data/genesis/integration-tests".as_ref();
        let rt_params =
            get_genesis_config::<DefaultContext, _>(&GenesisPaths::from_dir(integ_test_conf_dir))
                .unwrap();

        let chain_state = read_json_file(integ_test_conf_dir.join("chain_state.json")).unwrap();
        let kernel_params = BasicKernelGenesisConfig { chain_state };
        GenesisParams {
            runtime: rt_params,
            kernel: kernel_params,
        }
    };

    let (mut current_root, storage) = stf.init_chain(storage, demo_genesis_config);

    storage_manager
        .save_change_set(&genesis_block_header, storage)
        .expect("Saving genesis storage failed");
    storage_manager.finalize(&genesis_block_header).unwrap();

    // data generation
    let mut blobs = vec![];
    let mut blocks = vec![];
    for height in 1..=end_height {
        let filtered_block = MockBlock {
            header: MockBlockHeader::from_height(height),
            validity_cond: Default::default(),
            blobs: Default::default(),
        };
        let blob_txs = da_service.extract_relevant_blobs(&filtered_block);
        blocks.push(filtered_block);
        blobs.push(blob_txs);
    }

    // Setup. Block h=1 has a single tx that creates the token. Exclude from timers
    let filtered_block = blocks.remove(0);
    let storage = storage_manager
        .create_storage_on(filtered_block.header())
        .unwrap();
    let apply_block_result = stf.apply_slot(
        &current_root,
        storage,
        Default::default(),
        filtered_block.header(),
        &filtered_block.validity_cond,
        &mut blobs.remove(0),
    );
    current_root = apply_block_result.state_root;
    storage_manager
        .save_change_set(filtered_block.header(), apply_block_result.change_set)
        .unwrap();
    storage_manager.finalize(filtered_block.header()).unwrap();

    let mut data_to_commit = SlotCommit::new(filtered_block);
    data_to_commit.add_batch(apply_block_result.batch_receipts[0].clone());
    ledger_db.commit_slot(data_to_commit).unwrap();

    // 3 blocks to finalization
    let fork_length = 3;
    let blocks_num = blocks.len() as u64;
    // Rollup processing. Block h=2 -> end are the transfer transactions. Timers start here
    let total = Instant::now();
    let mut apply_block_time = Duration::new(0, 0);
    for (filtered_block, mut blobs) in blocks.into_iter().zip(blobs.into_iter()) {
        let storage = storage_manager
            .create_storage_on(filtered_block.header())
            .unwrap();
        let now = Instant::now();
        let apply_block_result = stf.apply_slot(
            &current_root,
            storage,
            Default::default(),
            filtered_block.header(),
            &filtered_block.validity_cond,
            &mut blobs,
        );
        apply_block_time += now.elapsed();
        h_apply_block.observe(now.elapsed().as_secs_f64());
        current_root = apply_block_result.state_root;
        storage_manager
            .save_change_set(filtered_block.header(), apply_block_result.change_set)
            .unwrap();

        if let Some(height_to_finalize) = filtered_block.header().height().checked_sub(fork_length)
        {
            // Blocks 0 & 1 has been finalized before
            if height_to_finalize > 1 {
                let header_to_finalize = MockBlockHeader::from_height(height_to_finalize);
                storage_manager.finalize(&header_to_finalize).unwrap();
            }
        }

        let mut data_to_commit = SlotCommit::new(filtered_block);
        for receipt in apply_block_result.batch_receipts {
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
            blocks_num,
            num_txns_per_block,
            num_success_txns,
        );
    }
    if prometheus_output {
        println!("{:#?}", registry.gather());
    }
    Ok(())
}
