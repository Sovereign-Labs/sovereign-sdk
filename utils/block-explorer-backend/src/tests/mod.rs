use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use axum_test::TestServer;
use demo_stf::app::App;
use demo_stf::genesis_config::get_genesis_config;
use serde_json::Value;
use sov_db::ledger_db::{LedgerDB, SlotCommit};
use sov_risc0_adapter::host::Risc0Verifier;
use sov_rng_da_service::{RngDaService, RngDaSpec};
use sov_rollup_interface::mocks::{MockAddress, MockBlock, MockBlockHeader};
use sov_rollup_interface::services::da::DaService;
use sov_rollup_interface::stf::StateTransitionFunction;
use sov_stf_runner::RollupConfig;
use sqlx::{Pool, Postgres};
use tempfile::TempDir;

use crate::api_v0::default_pagination_size;
use crate::db::Db;
use crate::indexer::index_blocks;
use crate::AppStateInner;

fn populate_ledger_db() -> LedgerDB {
    let start_height: u64 = 0u64;
    let mut end_height: u64 = 100u64;

    let mut rollup_config: RollupConfig<sov_celestia_adapter::DaServiceConfig> =
        toml::from_str(include_str!("rollup_config.toml"))
            .context("Failed to read rollup configuration")
            .unwrap();

    let temp_dir = TempDir::new().expect("Unable to create temporary directory");
    rollup_config.storage.path = PathBuf::from(temp_dir.path());
    let ledger_db =
        LedgerDB::with_path(&rollup_config.storage.path).expect("Ledger DB failed to open");

    let da_service = Arc::new(RngDaService::default());

    let demo_runner = App::<Risc0Verifier, RngDaSpec>::new(rollup_config.storage);

    let mut demo = demo_runner.stf;
    let sequencer_da_address = MockAddress::from(RngDaService::SEQUENCER_DA_ADDRESS);
    let demo_genesis_config = get_genesis_config(
        sequencer_da_address,
        #[cfg(feature = "experimental")]
        Default::default(),
    );

    let mut current_root = demo.init_chain(demo_genesis_config);

    // data generation
    let mut blobs = vec![];
    let mut blocks = vec![];
    for height in start_height..end_height {
        let num_bytes = height.to_le_bytes();
        let mut barray = [0u8; 32];
        barray[..num_bytes.len()].copy_from_slice(&num_bytes);
        let filtered_block = MockBlock {
            header: MockBlockHeader {
                hash: barray.into(),
                prev_hash: [0u8; 32].into(),
                height,
            },
            validity_cond: Default::default(),
            blobs: Default::default(),
        };
        blocks.push(filtered_block.clone());

        let blob_txs = da_service.extract_relevant_txs(&filtered_block);
        blobs.push(blob_txs.clone());
    }

    let mut height = 0u64;

    while height < end_height {
        let filtered_block = &blocks[height as usize];

        let mut data_to_commit = SlotCommit::new(filtered_block.clone());
        let apply_block_result = demo.apply_slot(
            &current_root,
            Default::default(),
            &filtered_block.header,
            &filtered_block.validity_cond,
            &mut blobs[height as usize],
        );
        current_root = apply_block_result.state_root;
        for receipts in apply_block_result.batch_receipts {
            data_to_commit.add_batch(receipts);
        }

        ledger_db.commit_slot(data_to_commit).unwrap();
        height += 1;
    }

    ledger_db
}

async fn create_test_server(pool: Pool<Postgres>) -> TestServer {
    let ledger_db = populate_ledger_db();
    let app_state = Arc::new(AppStateInner {
        db: Db { pool },
        rpc: ledger_db,
        base_url: "http://localhost:3010".to_string(),
    });
    index_blocks(app_state.clone(), Duration::default()).await;
    let service = crate::api_v0::router(app_state).into_make_service();
    TestServer::new(service).unwrap()
}

fn is_sorted<T>(iter: &[T]) -> bool
where
    T: Ord,
{
    iter.windows(2).all(|pair| pair[0] <= pair[1])
}

#[sqlx::test]
async fn transactions_default_pagination_size(pool: Pool<Postgres>) {
    let txs_response = create_test_server(pool)
        .await
        .get("/transactions")
        .await
        .json::<serde_json::Value>();
    let txs = txs_response["data"].as_array().unwrap();

    assert_eq!(txs.len(), default_pagination_size() as usize);
}

#[sqlx::test]
async fn max_pagination_size_is_respected(pool: Pool<Postgres>) {
    let txs_response = create_test_server(pool)
        .await
        .get("/transactions")
        .add_query_param("page[size]", "10000")
        .await
        .json::<serde_json::Value>();

    assert_ne!(txs_response["errors"].as_array().unwrap(), &[] as &[Value]);
}

#[sqlx::test]
async fn blocks(pool: Pool<Postgres>) {
    let blocks_response = create_test_server(pool)
        .await
        .get("/blocks")
        .await
        .json::<serde_json::Value>();
    let blocks = blocks_response["data"].as_array().unwrap();

    assert!(is_sorted(
        &blocks
            .iter()
            .map(|block| { block["number"].as_str().unwrap().parse::<u64>().unwrap() })
            .collect::<Vec<u64>>()
    ));
}
