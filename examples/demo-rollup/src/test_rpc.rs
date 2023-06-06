use std::net::SocketAddr;

use curl::easy::{Easy2, Handler, List, WriteError};
use demo_stf::app::{DemoBatchReceipt, DemoTxReceipt};
use serde::{Serialize, Deserialize};
use sov_db::ledger_db::{LedgerDB, SlotCommit};
use sov_rollup_interface::{traits::{CanonicalHash, BlockHeaderTrait}, services::da::SlotData, stf::{BatchReceipt, TransactionReceipt, Event, EventKey}};
use tokio::{sync::oneshot};

use crate::{config::RpcConfig, ledger_rpc};

struct Collector(Vec<u8>);

impl Handler for Collector {
    fn write(&mut self, data: &[u8]) -> Result<usize, WriteError> {
        self.0.extend_from_slice(data);
        Ok(data.len())
    }
}

fn query_test_helper(data: &[u8], expected: &str) {
    let mut headers = List::new();
    headers.append("Content-Type: application/json").unwrap();

    let mut easy = Easy2::new(Collector(Vec::new()));
    easy.http_headers(headers).unwrap();
    easy.post_fields_copy(data).unwrap();
    easy.post(true).unwrap();

    easy.url("http://127.0.0.1:12345").unwrap();
    easy.perform().unwrap();

    assert_eq!(easy.response_code().unwrap(), 200);
    let contents = easy.get_ref();
    assert_eq!(String::from_utf8_lossy(&contents.0), expected);
}

#[derive(Serialize, Deserialize, PartialEq, core::fmt::Debug, Clone)]
struct TestBlockHeader {
    prev_hash: [u8; 32],
}

impl CanonicalHash for TestBlockHeader {
    type Output = [u8; 32];

    fn hash(&self) -> Self::Output {
        self.prev_hash
    }
}

impl BlockHeaderTrait for TestBlockHeader {
    type Hash = [u8; 32];

    fn prev_hash(&self) -> Self::Hash {
        self.prev_hash
    }
}

#[derive(Serialize, Deserialize, PartialEq, core::fmt::Debug, Clone)]
struct TestBlock {
    curr_hash: [u8; 32],
    header: TestBlockHeader,
}

impl SlotData for TestBlock {
    type BlockHeader = TestBlockHeader;
    fn hash(&self) -> [u8; 32] {
        self.curr_hash
    }

    fn header(&self) -> &Self::BlockHeader {
        &self.header
    }
}

fn populate_ledger(ledger_db: &mut LedgerDB) -> () {

    let mut slot: SlotCommit<TestBlock, i32, i32> = SlotCommit::new(TestBlock {
        curr_hash: blake3::hash(b"slot_data").into(),
        header: TestBlockHeader {
            prev_hash: (blake3::hash(b"prev_header").into()),
        },
    });

    slot.add_batch(BatchReceipt {
        batch_hash: blake3::hash(b"batch_receipt").into(),
        tx_receipts: vec![TransactionReceipt::<i32> {
            tx_hash: blake3::hash(b"tx1").into(),
            body_to_save: Some(b"tx1 body".to_vec()),
            events: vec![],
            receipt: 0,
        }, 
       TransactionReceipt::<i32> {
            tx_hash: blake3::hash(b"tx2").into(),
            body_to_save: Some(b"tx2 body".to_vec()),
            events: vec![Event::new("event1_key", "event1_value"), Event::new("event2_key", "event2_value")],
            receipt: 1,
        } ],
        inner: 0,
    });

    slot.add_batch(BatchReceipt {
        batch_hash: blake3::hash(b"batch_receipt2").into(),
        tx_receipts: vec![TransactionReceipt::<i32> {
            tx_hash: blake3::hash(b"tx1").into(),
            body_to_save: Some(b"tx1 body".to_vec()),
            events: vec![],
            receipt: 0,
        }],
        inner: 1,
    });

    ledger_db.commit_slot(slot).unwrap()
}

// These tests reproduce the README workflow for the ledger_rpc, ie:
// - It creates and populate a simple ledger with a few transactions
// - It initializes the rpc server
// - It successively calls the different rpc methods registered and tests the answer
#[test]
fn simple_test_rpc() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_io()
        .enable_time()
        .build()
        .unwrap();

    rt.block_on(async {
        let (tx_start, rx_start) = oneshot::channel();
        let (tx_end, rx_end) = oneshot::channel();
        let rpc_config = RpcConfig {
            bind_host: "127.0.0.1".to_string(),
            bind_port: 12345,
        };

        let address = SocketAddr::new(rpc_config.bind_host.parse().unwrap(), rpc_config.bind_port);

        // Initialize the ledger database, which stores blocks, transactions, events, etc.
        let mut ledger_db = LedgerDB::temporary();

        populate_ledger(&mut ledger_db);

        let ledger_rpc_module =
            ledger_rpc::get_ledger_rpc::<i32, i32>(ledger_db.clone());

        rt.spawn(async move {
            let server = jsonrpsee::server::ServerBuilder::default()
                .build([address].as_ref())
                .await
                .unwrap();
            let _server_handle = server.start(ledger_rpc_module).unwrap();
            tx_start.send("server started").unwrap();
            rx_end.await.unwrap();
        });
    
        rx_start.await.unwrap();
    
        let data = r#"{"jsonrpc":"2.0","method":"ledger_getHead","params":[],"id":1}"#.as_bytes();
        let expected = r#"{"jsonrpc":"2.0","result":{"number":1,"hash":"0x75600b5af42511f76ae5bc4f2c884a0b8824d4617402adc9ff2320adf73a0d31","batch_range":{"start":1,"end":3}},"id":1}"#;
        
        query_test_helper(data, expected);

        let data = r#"{"jsonrpc":"2.0","method":"ledger_getTransactions","params":[[{ "batch_id": 1, "offset": 0}]],"id":1}"#
            .as_bytes();
        let expected = r#"{"jsonrpc":"2.0","result":[{"hash":"0x329c58b0a9b08973bed32452c2cefa0ab567146505711337c955b24cf41c6e99","event_range":{"start":1,"end":1},"body":[116,120,49,32,98,111,100,121],"custom_receipt":0}],"id":1}"#;
        query_test_helper(data, expected);

        let data = r#"{"jsonrpc":"2.0","method":"ledger_getTransactions","params":[[{ "batch_id": 1, "offset": 1}]],"id":1}"#
            .as_bytes();
        let expected = r#"{"jsonrpc":"2.0","result":[{"hash":"0xae87c50974dc43f4f70e84cb27e4630e4e47f782cff1e3d484310d82cea9acf6","event_range":{"start":1,"end":3},"body":[116,120,50,32,98,111,100,121],"custom_receipt":1}],"id":1}"#;
        query_test_helper(data, expected);
        
        
        let data = r#"{"jsonrpc":"2.0","method":"ledger_getBatches","params":[[2], "Standard"],"id":1}"#
            .as_bytes();
        let expected = r#"{"jsonrpc":"2.0","result":[{"hash":"0x41c8790eb95a24d1b4aabc606e88c602073c72ada51ebc72300a82591dc49459","tx_range":{"start":3,"end":4},"txs":["0x329c58b0a9b08973bed32452c2cefa0ab567146505711337c955b24cf41c6e99"],"custom_receipt":1}],"id":1}"#;
        query_test_helper(data, expected);

        let data = r#"{"jsonrpc":"2.0","method":"ledger_getBatches","params":[[1], "Compact"],"id":1}"#
            .as_bytes();
        let expected = r#"{"jsonrpc":"2.0","result":[{"hash":"0xf344c1da53b56f7a49d02ee20899fb914bdfc3632db4f337ade196e72f5eb083","tx_range":{"start":1,"end":3},"custom_receipt":0}],"id":1}"#;
        query_test_helper(data, expected);

        let data = r#"{"jsonrpc":"2.0","method":"ledger_getBatches","params":[[0], "Compact"],"id":1}"#
            .as_bytes();
        let expected = r#"{"jsonrpc":"2.0","result":[null],"id":1}"#;
        query_test_helper(data, expected);

        let data = r#"{"jsonrpc":"2.0","method":"ledger_getEvents","params":[1],"id":1}"#.as_bytes();
        let expected = r#"{"jsonrpc":"2.0","result":[{"key":[101,118,101,110,116,49,95,107,101,121],"value":[101,118,101,110,116,49,95,118,97,108,117,101]}],"id":1}"#;
        query_test_helper(data, expected);

        let data = r#"{"jsonrpc":"2.0","method":"ledger_getEvents","params":[2],"id":1}"#.as_bytes();
        let expected = r#"{"jsonrpc":"2.0","result":[{"key":[101,118,101,110,116,50,95,107,101,121],"value":[101,118,101,110,116,50,95,118,97,108,117,101]}],"id":1}"#;
        query_test_helper(data, expected);

        let data = r#"{"jsonrpc":"2.0","method":"ledger_getEvents","params":[3],"id":1}"#.as_bytes();
        let expected = r#"{"jsonrpc":"2.0","result":[null],"id":1}"#;
        query_test_helper(data, expected);


        tx_end.send("drop server").unwrap();
    });

}
