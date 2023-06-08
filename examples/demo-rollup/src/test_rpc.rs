use reqwest::header::CONTENT_TYPE;
use sov_db::ledger_db::{LedgerDB, SlotCommit};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;

#[cfg(test)]
use sov_rollup_interface::mocks::{TestBlock, TestBlockHeader};

use sov_rollup_interface::stf::{BatchReceipt, Event, TransactionReceipt};
use tendermint::crypto::Sha256;
use tokio::sync::oneshot;

use crate::{config::RpcConfig, ledger_rpc};

async fn query_test_helper(data: String, expected: &str, rpc_config: RpcConfig) {
    let (addr, port) = (rpc_config.bind_host, rpc_config.bind_port);
    let client = reqwest::Client::new();
    let url_str = format!("http://{addr}:{port}");

    let res = client
        .post(url_str)
        .header(CONTENT_TYPE, "application/json")
        .body(data)
        .send()
        .await
        .unwrap();

    assert_eq!(res.status().as_u16(), 200);
    let contents = res.text().await.unwrap();
    assert_eq!((&contents), expected);
}

fn populate_ledger(ledger_db: &mut LedgerDB) -> () {
    let mut slot: SlotCommit<TestBlock, i32, i32> = SlotCommit::new(TestBlock {
        curr_hash: sha2::Sha256::digest(b"slot_data").into(),
        header: TestBlockHeader {
            prev_hash: (sha2::Sha256::digest(b"prev_header").into()),
        },
    });

    slot.add_batch(BatchReceipt {
        batch_hash: ::sha2::Sha256::digest(b"batch_receipt").into(),
        tx_receipts: vec![
            TransactionReceipt::<i32> {
                tx_hash: ::sha2::Sha256::digest(b"tx1").into(),
                body_to_save: Some(b"tx1 body".to_vec()),
                events: vec![],
                receipt: 0,
            },
            TransactionReceipt::<i32> {
                tx_hash: ::sha2::Sha256::digest(b"tx2").into(),
                body_to_save: Some(b"tx2 body".to_vec()),
                events: vec![
                    Event::new("event1_key", "event1_value"),
                    Event::new("event2_key", "event2_value"),
                ],
                receipt: 1,
            },
        ],
        inner: 0,
    });

    slot.add_batch(BatchReceipt {
        batch_hash: ::sha2::Sha256::digest(b"batch_receipt2").into(),
        tx_receipts: vec![TransactionReceipt::<i32> {
            tx_hash: ::sha2::Sha256::digest(b"tx1").into(),
            body_to_save: Some(b"tx1 body".to_vec()),
            events: vec![],
            receipt: 0,
        }],
        inner: 1,
    });

    ledger_db.commit_slot(slot).unwrap()
}

fn test_helper(data: String, expected: &str) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_io()
        .enable_time()
        .build()
        .unwrap();

    rt.block_on(async {
        let (tx_start, rx_start) = oneshot::channel();
        let (tx_end, rx_end) = oneshot::channel();

        let address = SocketAddr::new("127.0.0.1".parse().unwrap(), 0);

        // Initialize the ledger database, which stores blocks, transactions, events, etc.
        let tmpdir = tempfile::tempdir().unwrap();
        let mut ledger_db = LedgerDB::with_path(tmpdir.path()).unwrap();

        populate_ledger(&mut ledger_db);

        let ledger_rpc_module = ledger_rpc::get_ledger_rpc::<i32, i32>(ledger_db.clone());

        let actual_port_placeholder: Arc<AtomicU16> = Arc::new(AtomicU16::new(0));
        let actual_port_placeholder_clone = actual_port_placeholder.clone();
        rt.spawn(async move {
            let server = jsonrpsee::server::ServerBuilder::default()
                .build([address].as_ref())
                .await
                .unwrap();
            let actual_address = server.local_addr().unwrap();
            actual_port_placeholder_clone.store(actual_address.port(), Ordering::Relaxed);
            let _server_handle = server.start(ledger_rpc_module).unwrap();
            tx_start.send("server started").unwrap();
            rx_end.await.unwrap();
        });

        rx_start.await.unwrap();

        for _ in 0..1000 {
            if actual_port_placeholder.load(Ordering::Relaxed) != 0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        let rpc_config = RpcConfig {
            bind_host: "127.0.0.1".to_string(),
            bind_port: actual_port_placeholder.load(Ordering::Relaxed),
        };

        query_test_helper(data, expected, rpc_config).await;

        tx_end.send("drop server").unwrap();
    });
}

// These tests reproduce the README workflow for the ledger_rpc, ie:
// - It creates and populate a simple ledger with a few transactions
// - It initializes the rpc server
// - It successively calls the different rpc methods registered and tests the answer
// Side note: we need to change the port for each test to avoid concurrent access issues
#[test]
fn test_get_head() {
    let data = r#"{"jsonrpc":"2.0","method":"ledger_getHead","params":[],"id":1}"#.to_string();
    let expected = r#"{"jsonrpc":"2.0","result":{"number":1,"hash":"0xd1231a38586e68d0405dc55ae6775e219f29fff1f7e0c6410d0ac069201e550b","batch_range":{"start":1,"end":3}},"id":1}"#;

    test_helper(data, expected);
}

#[test]
fn test_get_transactions() {
    let data = r#"{"jsonrpc":"2.0","method":"ledger_getTransactions","params":[[{ "batch_id": 1, "offset": 0}]],"id":1}"#.to_string();
    let expected = r#"{"jsonrpc":"2.0","result":[{"hash":"0x709b55bd3da0f5a838125bd0ee20c5bfdd7caba173912d4281cae816b79a201b","event_range":{"start":1,"end":1},"body":[116,120,49,32,98,111,100,121],"custom_receipt":0}],"id":1}"#;
    test_helper(data, expected);

    let data = r#"{"jsonrpc":"2.0","method":"ledger_getTransactions","params":[[{ "batch_id": 1, "offset": 1}]],"id":1}"#
            .to_string();
    let expected = r#"{"jsonrpc":"2.0","result":[{"hash":"0x27ca64c092a959c7edc525ed45e845b1de6a7590d173fd2fad9133c8a779a1e3","event_range":{"start":1,"end":3},"body":[116,120,50,32,98,111,100,121],"custom_receipt":1}],"id":1}"#;
    test_helper(data, expected);
}

#[test]
fn test_get_batches() {
    let data =
        r#"{"jsonrpc":"2.0","method":"ledger_getBatches","params":[[2], "Standard"],"id":1}"#
            .to_string();
    let expected = r#"{"jsonrpc":"2.0","result":[{"hash":"0xf85fe0cb36fdaeca571c896ed476b49bb3c8eff00d935293a8967e1e9a62071e","tx_range":{"start":3,"end":4},"txs":["0x709b55bd3da0f5a838125bd0ee20c5bfdd7caba173912d4281cae816b79a201b"],"custom_receipt":1}],"id":1}"#;
    test_helper(data, expected);

    let data = r#"{"jsonrpc":"2.0","method":"ledger_getBatches","params":[[1], "Compact"],"id":1}"#
        .to_string();
    let expected = r#"{"jsonrpc":"2.0","result":[{"hash":"0xb5515a80204963f7db40e98af11aedb49a394b1c7e3d8b5b7a33346b8627444f","tx_range":{"start":1,"end":3},"custom_receipt":0}],"id":1}"#;
    test_helper(data, expected);

    let data = r#"{"jsonrpc":"2.0","method":"ledger_getBatches","params":[[0], "Compact"],"id":1}"#
        .to_string();
    let expected = r#"{"jsonrpc":"2.0","result":[null],"id":1}"#;
    test_helper(data, expected);
}

#[test]
fn test_get_events() {
    let data = r#"{"jsonrpc":"2.0","method":"ledger_getEvents","params":[1],"id":1}"#.to_string();
    let expected = r#"{"jsonrpc":"2.0","result":[{"key":[101,118,101,110,116,49,95,107,101,121],"value":[101,118,101,110,116,49,95,118,97,108,117,101]}],"id":1}"#;
    test_helper(data, expected);

    let data = r#"{"jsonrpc":"2.0","method":"ledger_getEvents","params":[2],"id":1}"#.to_string();
    let expected = r#"{"jsonrpc":"2.0","result":[{"key":[101,118,101,110,116,50,95,107,101,121],"value":[101,118,101,110,116,50,95,118,97,108,117,101]}],"id":1}"#;
    test_helper(data, expected);

    let data = r#"{"jsonrpc":"2.0","method":"ledger_getEvents","params":[3],"id":1}"#.to_string();
    let expected = r#"{"jsonrpc":"2.0","result":[null],"id":1}"#;
    test_helper(data, expected);
}
