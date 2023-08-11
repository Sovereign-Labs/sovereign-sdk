use jupiter::types::FilteredCelestiaBlock;
use borsh::de::BorshDeserialize;
use std::fs::read_to_string;
use std::path::PathBuf;
use std::io::Write;
use std::fs::File;
use std::env;
use tempfile::TempDir;
use sov_modules_api::PrivateKey;

use anyhow::Context;
use const_rollup_config::{ROLLUP_NAMESPACE_RAW, SEQUENCER_DA_ADDRESS};
use demo_stf::app::{DefaultPrivateKey, App};
use demo_stf::genesis_config::create_demo_genesis_config;
use jupiter::da_service::{CelestiaService, DaServiceConfig};
use jupiter::types::NamespaceId;
use jupiter::verifier::RollupParams;
use jupiter::BlobWithSender;
use methods::{ROLLUP_ELF};
use risc0_adapter::host::Risc0Host;
use serde::Deserialize;
use sov_rollup_interface::services::da::DaService;
use sov_rollup_interface::stf::StateTransitionFunction;
use sov_rollup_interface::zk::ZkvmHost;
use sov_stf_runner::{from_toml_path, Config as RunnerConfig};
use std::fs::OpenOptions;
use std::sync::{Arc, Mutex};
use std::path::Path;
use std::fs::remove_file;

use log4rs::{
    config::{Appender, Config, Root},
};
use regex::Regex;

#[derive(Debug)]
struct RegexAppender {
    regex: Regex,
    file: Arc<Mutex<File>>
}

impl RegexAppender {
    fn new(pattern: &str, file_path: &str) -> Self {
        if Path::new(file_path).exists() {
            remove_file(file_path).expect("Failed to remove existing file");
        }
        let file = Arc::new(Mutex::new(OpenOptions::new().create(true).append(true).open(file_path).unwrap()));
        let regex = Regex::new(pattern).unwrap();
        RegexAppender { regex, file }
    }
}

impl log::Log for RegexAppender {

    fn log(&self, record: &log::Record) {
        if let Some(captures) = self.regex.captures(record.args().to_string().as_str()) {
            if let Some(matched_pc) = captures.get(1) {
                let pc_value_num = u64::from_str_radix(&matched_pc.as_str()[2..], 16).unwrap();
                let pc_value =  format!("{}\n",pc_value_num);
                let mut file_guard = self.file.lock().unwrap();
                file_guard.write_all(pc_value.as_bytes()).unwrap();
            }
        }
    }

    fn enabled(&self, _metadata: &log::Metadata) -> bool {
        true
    }

    fn flush(&self) {
    }
}

fn get_config(rollup_trace: &str) -> Config {
    let regex_pattern = r".*?pc: (0x[0-9a-fA-F]+), insn.*";
    // let log_file = "/Users/dubbelosix/sovereign/examples/demo-prover/matched_pattern.log";

    let custom_appender = RegexAppender::new(regex_pattern, rollup_trace);

    Config::builder()
        .appender(Appender::builder().build("custom_appender", Box::new(custom_appender)))
        .build(Root::builder().appender("custom_appender").build(log::LevelFilter::Trace))
        .unwrap()
}

#[cfg(feature = "bench")]
use risc0_adapter::metrics::GLOBAL_HASHMAP;

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
    if let Some(rollup_trace) = env::var("ROLLUP_TRACE").ok() {
        if let Err(e) = log4rs::init_config(get_config(&rollup_trace)) {
            eprintln!("Error initializing logger: {:?}", e);
        }
    }

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

    let app: App<Risc0Host, BlobWithSender> =
        App::new(rollup_config.runner.storage.clone());

    let mut demo = app.stf;

    let genesis_config = create_demo_genesis_config(
        100000000,
        sequencer_private_key.default_address(),
        SEQUENCER_DA_ADDRESS.to_vec(),
        &sequencer_private_key,
        &sequencer_private_key,
    );
    println!("Starting from empty storage, initialization chain");
    demo.init_chain(genesis_config);

    let mut prev_state_root = {
        let res = demo.apply_slot(Default::default(), []);
        res.state_root.0
    };

    let hex_data = read_to_string("benches/blocks.hex").expect("Failed to read data");
    let borshed_blocks: Vec<FilteredCelestiaBlock> = hex_data
        .lines()
        .map(|line| {
            let bytes = hex::decode(line).expect("Failed to decode hex data");
            FilteredCelestiaBlock::try_from_slice(&bytes).expect("Failed to deserialize data")
        })
        .collect();

    for height in 2..(borshed_blocks.len() as u64) {
        num_blocks+=1;
        let mut host = Risc0Host::new(ROLLUP_ELF);
        host.write_to_guest(prev_state_root);
        println!(
            "Requesting data for height {} and prev_state_root 0x{}",
            height,
            hex::encode(prev_state_root)
        );
        let filtered_block = &borshed_blocks[height as usize];
        let _header_hash = hex::encode(filtered_block.header.header.hash());
        host.write_to_guest(&filtered_block.header);
        let (mut blob_txs, inclusion_proof, completeness_proof) =
            da_service.extract_relevant_txs_with_proof(&filtered_block).await;

        host.write_to_guest(&inclusion_proof);
        host.write_to_guest(&completeness_proof);
        host.write_to_guest(&blob_txs);

        if !blob_txs.is_empty() {
            num_blobs+=blob_txs.len();
        }
        let result = demo.apply_slot(Default::default(), &mut blob_txs);
        for r in result.batch_receipts {
            let num_tx = r.tx_receipts.len();
            num_total_transactions+=num_tx;
            if num_tx > 0 {
                num_blocks_with_txns+=1;
            }
        }
        // println!("{:?}",result.batch_receipts);

        host.write_to_guest(&result.witness);

        println!("Skipping prover at block {height} to capture cycle counts\n");
        let _receipt = host.run_without_proving().expect("Prover should run successfully");
        println!("==================================================\n");
        prev_state_root = result.state_root.0;

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
