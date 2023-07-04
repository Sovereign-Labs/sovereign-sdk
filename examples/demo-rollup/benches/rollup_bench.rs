use anyhow::Context;
use demo_stf::app::NativeAppRunner;
use demo_stf::runner_config::from_toml_path;
use std::env;
use std::sync::Arc;

use sov_demo_rollup::config::RollupConfig;
use sov_demo_rollup::rng_xfers::RngDaService;

use risc0_adapter::host::Risc0Verifier;
use sov_db::ledger_db::{LedgerDB, SlotCommit};
use sov_rollup_interface::mocks::{TestBlock, TestBlockHeader, TestHash};
use sov_rollup_interface::services::stf_runner::StateTransitionRunner;

use const_rollup_config::SEQUENCER_DA_ADDRESS;
use demo_stf::genesis_config::create_demo_genesis_config;
use sov_modules_api::default_signature::private_key::DefaultPrivateKey;
use sov_rollup_interface::services::da::DaService;
use sov_rollup_interface::stf::StateTransitionFunction;
use std::fs;
use std::io;

use criterion::{criterion_group, criterion_main, Criterion};
use std::time::Duration;

fn remove_dir_if_exists<P: AsRef<std::path::Path>>(path: P) -> io::Result<()> {
    if path.as_ref().exists() {
        fs::remove_dir_all(&path)
    } else {
        Ok(())
    }
}

fn rollup_bench(_bench: &mut Criterion) {
    let start_height: u64 = 0u64;
    let mut end_height: u64 = 100u64;
    if let Ok(val) = env::var("BLOCKS") {
        end_height = val.parse().unwrap();
    }

    let mut c = Criterion::default()
        .sample_size(10)
        .measurement_time(Duration::from_secs(20));
    let rollup_config_path = "benches/rollup_config.toml".to_string();
    let rollup_config: RollupConfig = from_toml_path(&rollup_config_path)
        .context("Failed to read rollup configuration")
        .unwrap();

    remove_dir_if_exists(&rollup_config.runner.storage.path).unwrap();
    let ledger_db =
        LedgerDB::with_path(&rollup_config.runner.storage.path).expect("Ledger DB failed to open");

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
    for height in start_height..end_height {
        let num_bytes = height.to_le_bytes();
        let mut barray = [0u8; 32];
        barray[..num_bytes.len()].copy_from_slice(&num_bytes);
        let filtered_block = if height == 0 {
            TestBlock {
                curr_hash: barray,
                header: TestBlockHeader {
                    prev_hash: TestHash([0u8; 32]),
                },
                height,
            }
        } else {
            TestBlock {
                curr_hash: barray,
                header: TestBlockHeader {
                    prev_hash: TestHash([0u8; 32]),
                },
                height,
            }
        };
        blocks.push(filtered_block.clone());

        let blob_txs = da_service.extract_relevant_txs(&filtered_block);
        blobs.push(blob_txs.clone());
    }

    let mut height = 0u64;
    c.bench_function("rollup main loop", |b| {
        b.iter(|| {
            let filtered_block = &blocks[height as usize];

            let mut data_to_commit = SlotCommit::new(filtered_block.clone());
            demo.begin_slot(Default::default());

            for blob in &mut blobs[height as usize] {
                let receipts = demo.apply_blob(blob, None);
                // println!("{:?}", receipts);
                data_to_commit.add_batch(receipts);
            }
            let (_next_state_root, _witness) = demo.end_slot();

            ledger_db.commit_slot(data_to_commit).unwrap();
            height += 1;
        })
    });
}

criterion_group!(benches, rollup_bench);
criterion_main!(benches);
