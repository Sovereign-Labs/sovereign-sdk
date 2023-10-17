use std::cmp::Ordering;
use std::fmt::Display;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum_test::TestServer;
use demo_stf::genesis_config::{get_genesis_config, GenesisPaths};
use demo_stf::App;
use jsonrpsee::ws_client::WsClientBuilder;
use serde_json::Value;
use sov_db::ledger_db::{LedgerDB, SlotCommit};
use sov_modules_stf_template::{SequencerOutcome, TxEffect};
use sov_risc0_adapter::host::Risc0Verifier;
use sov_rng_da_service::{RngDaService, RngDaSpec};
use sov_rollup_interface::mocks::{MockAddress, MockBlock, MockBlockHeader, MockHash};
use sov_rollup_interface::services::da::DaService;
use sov_rollup_interface::stf::StateTransitionFunction;
use testcontainers::clients::Cli;
use testcontainers::Container;
use testcontainers_modules::postgres::Postgres as PostgresImage;

use crate::api_v0::default_pagination_size;
use crate::db::Db;
use crate::indexer::index_blocks;
use crate::AppStateInner;

type PostgresContainer<'a> = Container<'a, PostgresImage>;

fn populate_ledger_db() -> LedgerDB {
    std::env::set_var("TXNS_PER_BLOCK", "10");

    let start_height: u64 = 0u64;
    let end_height: u64 = 99u64;

    let temp_dir = tempfile::tempdir().unwrap();
    let rollup_config = sov_state::config::Config {
        path: PathBuf::from(temp_dir.path()),
    };

    let ledger_db = LedgerDB::with_path(&rollup_config.path).expect("Ledger DB failed to open");

    let da_service = Arc::new(RngDaService);

    let demo_runner = App::<Risc0Verifier, RngDaSpec>::new(rollup_config);

    let mut demo = demo_runner.stf;
    let sequencer_da_address = MockAddress { addr: [0u8; 32] };
    let demo_genesis_config = get_genesis_config(
        sequencer_da_address,
        &GenesisPaths::from_dir("../../examples/test-data/genesis/integration-tests"),
        Default::default(),
    );

    let mut current_root = demo.init_chain(demo_genesis_config);

    // data generation
    let mut blobs = vec![];
    let mut blocks = vec![];
    let mut prev_hash;
    for height in start_height..end_height {
        let num_bytes = height.to_le_bytes();
        let mut barray = [0u8; 32];
        barray[..num_bytes.len()].copy_from_slice(&num_bytes);
        prev_hash = barray;
        let filtered_block = MockBlock {
            header: MockBlockHeader {
                hash: barray.into(),
                prev_hash: MockHash(prev_hash),
                height,
            },
            validity_cond: Default::default(),
            blobs: Default::default(),
        };
        blocks.push(filtered_block.clone());

        let blob_txs = da_service.extract_relevant_blobs(&filtered_block);
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

async fn create_test_server(
    docker_cli: &Cli,
) -> (
    PostgresContainer,
    jsonrpsee::server::ServerHandle,
    TestServer,
) {
    use sov_ledger_rpc::server::rpc_module;

    type B = SequencerOutcome<MockAddress>;
    type Tx = TxEffect;

    let postgres_container = docker_cli.run(PostgresImage::default());
    let ledger_db = populate_ledger_db();
    let server = jsonrpsee::server::ServerBuilder::default()
        .build("127.0.0.1:0")
        .await
        .unwrap();
    let addr = server.local_addr().unwrap();
    let server_rpc_module = rpc_module::<LedgerDB, B, Tx>(ledger_db).unwrap();
    let server_handle = server.start(server_rpc_module);
    let rpc = Arc::new(
        WsClientBuilder::new()
            .build(format!("ws://{}", addr))
            .await
            .unwrap(),
    );
    let connection_string = &format!(
        "postgresql://postgres:postgres@127.0.0.1:{}/postgres?sslmode=disable",
        postgres_container.get_host_port_ipv4(5432)
    );
    let app_state = Arc::new(AppStateInner {
        db: Db::new(connection_string).await.unwrap(),
        rpc,
        base_url: "http://localhost:3010".to_string(),
    });
    index_blocks(app_state.clone(), Duration::default())
        .await
        .unwrap();
    let service = crate::api_v0::router(app_state).into_make_service();
    (
        postgres_container,
        server_handle,
        TestServer::new(service).unwrap(),
    )
}

fn is_sorted<T>(iter: &[T], desired_order: Ordering) -> bool
where
    T: Ord + Display,
{
    for pair in iter.windows(2) {
        let a = &pair[0];
        let b = &pair[1];
        if a.cmp(b) != desired_order {
            println!("{} and {} are not in the right order", a, b);
            return false;
        }
    }

    true
}

// FIX THESE TESTS
//async fn get_transactions_req(server: &TestServer, query: TransactionsQuery) -> TestResponse {}
//
//#[tokio::test]
//async fn get_transactions_query_combinations(pool: Pool<Postgres>) {
//    let (_, server) = create_test_server(pool).await;
//
//    get_transactions_req(&server, &[("page[selection]", "first")])
//        .await
//        .assert_status_ok();
//    get_transactions_req(&server, &[("page[selection]", "last")])
//        .await
//        .assert_status_ok();
//}

#[tokio::test]
#[ignore]
async fn events_first_page_is_full() {
    let docker_cli = Cli::default();
    let (_container, _, server) = create_test_server(&docker_cli).await;
    let response = server
        .get("/events")
        .add_query_param("page[size]", "142")
        .add_query_param("page[selection]", "first")
        .await;

    let json = response.json::<serde_json::Value>();
    assert_eq!(json.get("errors"), None);
    let events = json["data"].as_array().unwrap();
    assert_ne!(events.len(), 142usize);

    response.assert_status_ok();
}

#[tokio::test]
#[ignore]
async fn batches_not_empty() {
    let docker_cli = Cli::default();
    let (_container, _, server) = create_test_server(&docker_cli).await;
    let response = server.get("/batches").await;

    let json = response.json::<serde_json::Value>();
    assert_eq!(json.get("errors"), None);
    let batches = json["data"].as_array().unwrap();
    assert_ne!(batches, &[] as &[Value]);

    response.assert_status_ok();
}

#[tokio::test]
#[ignore]
async fn invalid_uri_returns_valid_json() {
    let docker_cli = Cli::default();
    let (_container, _, server) = create_test_server(&docker_cli).await;
    let response = server.get("/foobar-invalid-path").await;
    response.assert_status_not_found();
    let json = response.json::<serde_json::Value>();

    assert_ne!(json["errors"].as_array().unwrap(), &[] as &[Value]);
}

#[tokio::test]
#[ignore]
async fn initially_there_are_100_blocks() {
    let docker_cli = Cli::default();
    let (_container, _, server) = create_test_server(&docker_cli).await;
    let response = server
        .get("/blocks")
        .add_query_param("page[size]", "150")
        .await;

    let json = response.json::<serde_json::Value>();
    assert_eq!(json.get("errors"), None);
    //FIXME: this is broken as of now, there's missing blocks
    //assert_eq!(json["data"].as_array().unwrap().len(), 100);

    response.assert_status_ok();
}

#[tokio::test]
#[ignore]
async fn block_by_hash_bad_hexstring() {
    let docker_cli = Cli::default();
    let (_container, _, server) = create_test_server(&docker_cli).await;
    // Odd number of digits, which is invalid.
    let response = server.get("/blocks/0x123").await;
    response.assert_status_bad_request();
}

#[tokio::test]
#[ignore]
async fn block_404() {
    let docker_cli = Cli::default();
    let (_container, _, server) = create_test_server(&docker_cli).await;
    let response = server.get("/blocks/0x1234").await;

    response.assert_status_not_found();
    let json = response.json::<serde_json::Value>();

    assert_eq!(json.get("data"), None);
    assert_ne!(json["errors"].as_array().unwrap(), &[] as &[Value]);
}

#[tokio::test]
#[ignore]
async fn transactions_first_page_is_full() {
    let docker_cli = Cli::default();
    let (_container, _, server) = create_test_server(&docker_cli).await;
    let txs_response = server
        .get("/transactions")
        .await
        .json::<serde_json::Value>();
    let txs = txs_response["data"].as_array().unwrap();

    assert_eq!(txs.len(), default_pagination_size() as usize);
}

#[tokio::test]
#[ignore]
async fn max_pagination_size_is_respected() {
    let docker_cli = Cli::default();
    let (_container, _, server) = create_test_server(&docker_cli).await;
    let txs_response = server
        .get("/transactions")
        .add_query_param("page[size]", "10000")
        .await
        .json::<serde_json::Value>();

    assert_ne!(txs_response["errors"].as_array().unwrap(), &[] as &[Value]);
}

#[tokio::test]
#[ignore]
async fn blocks_default_order() {
    let docker_cli = Cli::default();
    let (_container, _, server) = create_test_server(&docker_cli).await;
    let blocks_response = server.get("/blocks").await.json::<serde_json::Value>();
    let blocks = blocks_response["data"].as_array().unwrap();
    let block_numbers = blocks
        .iter()
        .map(|block_response_obj| {
            block_response_obj["attributes"]["number"]
                .as_i64()
                .unwrap_or_else(|| panic!("Failed to get block number from {}", block_response_obj))
        })
        .collect::<Vec<i64>>();

    // Blocks are sorted in descending order by default.
    assert!(is_sorted(&block_numbers, Ordering::Greater));
}

#[tokio::test]
#[ignore]
async fn blocks_descending_order() {
    let docker_cli = Cli::default();
    let (_container, _, server) = create_test_server(&docker_cli).await;
    let blocks_response = server
        .get("/blocks")
        .add_query_param("sort", "-number")
        .await
        .json::<serde_json::Value>();
    let blocks = blocks_response["data"].as_array().unwrap();
    let block_numbers = blocks
        .iter()
        .map(|block_response_obj| {
            block_response_obj["attributes"]["number"]
                .as_i64()
                .unwrap_or_else(|| panic!("Failed to get block number from {}", block_response_obj))
        })
        .collect::<Vec<i64>>();

    // Blocks are sorted in descending order by default.
    assert!(is_sorted(&block_numbers, Ordering::Greater));
}

#[tokio::test]
#[ignore]
async fn blocks_ascending_order() {
    let docker_cli = Cli::default();
    let (_container, _, server) = create_test_server(&docker_cli).await;
    let blocks_response = server
        .get("/blocks")
        .add_query_param("sort", "number")
        .await
        .json::<serde_json::Value>();
    let blocks = blocks_response["data"].as_array().unwrap();
    let block_numbers = blocks
        .iter()
        .map(|block_response_obj| {
            block_response_obj["attributes"]["number"]
                .as_i64()
                .unwrap_or_else(|| panic!("Failed to get block number from {}", block_response_obj))
        })
        .collect::<Vec<i64>>();

    assert!(is_sorted(&block_numbers, Ordering::Less));
}
