use std::env;
use std::sync::Arc;
use anyhow::Context;
use demo_stf::app::NativeAppRunner;
use demo_stf::runner_config::from_toml_path;

use sov_demo_rollup::config::RollupConfig;
use sov_demo_rollup::rng_xfers::RngDaService;

use jupiter::verifier::address::CelestiaAddress;
use risc0_adapter::host::Risc0Verifier;
use sov_db::ledger_db::{LedgerDB, SlotCommit};
use sov_rollup_interface::mocks::{TestBlob, TestBlock, TestBlockHeader, TestHash};
use sov_rollup_interface::services::stf_runner::StateTransitionRunner;

use std::fs;
use std::io;
use tracing::Level;
use tracing_subscriber::fmt::format;
use const_rollup_config::SEQUENCER_DA_ADDRESS;
use sov_rollup_interface::services::da::{DaService, SlotData};
use sov_rollup_interface::stf::StateTransitionFunction;
use demo_stf::genesis_config::create_demo_genesis_config;
use sov_modules_api::default_signature::private_key::DefaultPrivateKey;

use std::fs::File;


fn remove_dir_if_exists<P: AsRef<std::path::Path>>(path: P) -> io::Result<()> {
    if path.as_ref().exists() {
        fs::remove_dir_all(&path)
    } else {
        Ok(())
    }
}

const START_HEIGHT: u64 = 0u64;
const END_HEIGHT: u64 = 100u64;

fn main() {
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber)
        .map_err(|_err| eprintln!("Unable to set global default subscriber"))
        .expect("Cannot fail to set subscriber");

    let rollup_config_path =  "benches/rollup_config.toml".to_string();
    let rollup_config: RollupConfig =
        from_toml_path(&rollup_config_path).context("Failed to read rollup configuration").unwrap();

    remove_dir_if_exists(&rollup_config.runner.storage.path).unwrap();
    let ledger_db = LedgerDB::with_path(&rollup_config.runner.storage.path).expect("Ledger DB failed to open");

    let da_service = Arc::new(RngDaService::new());

    let mut demo_runner = NativeAppRunner::<Risc0Verifier>::new(rollup_config.runner.clone());

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
    for height in START_HEIGHT..END_HEIGHT {
        let num_bytes = height.to_le_bytes();
        let mut barray = [0u8; 32];
        barray[..num_bytes.len()].copy_from_slice(&num_bytes);
        let filtered_block = if height == 0 {
            TestBlock {
                curr_hash: barray,
                header: TestBlockHeader {
                    prev_hash: TestHash([0u8; 32]),
                },
                height
            }
        } else {
            TestBlock {
                curr_hash: barray,
                header: TestBlockHeader {
                    prev_hash: TestHash([0u8; 32]),
                },
                height
            }
        };
        blocks.push(filtered_block.clone());

        let mut blob_txs = da_service.extract_relevant_txs(&filtered_block);
        blobs.push(blob_txs.clone());
    }
    cargo_profiler::start().unwrap();
    for height in START_HEIGHT..END_HEIGHT {
        let num_bytes = height.to_le_bytes();
        let mut barray = [0u8; 32];
        barray[..num_bytes.len()].copy_from_slice(&num_bytes);
        let filtered_block = &blocks[height as usize];

        let mut data_to_commit = SlotCommit::new(filtered_block.clone());
        demo.begin_slot(Default::default());

        for blob in &mut blobs[height as usize] {
            let receipts = demo.apply_blob(blob, None);
            data_to_commit.add_batch(receipts);
        }
        let (_next_state_root, _witness) = demo.end_slot();

        ledger_db.commit_slot(data_to_commit).unwrap();
    }
    cargo_profiler::stop().unwrap();

    // f::dump_html(&mut File::create("demo-rollup.html").unwrap()).unwrap();

}