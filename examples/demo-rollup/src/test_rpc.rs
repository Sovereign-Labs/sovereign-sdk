use std::{
    collections::hash_map::DefaultHasher,
    hash::{self, Hash, Hasher},
    net::SocketAddr,
    path::PathBuf,
    time::Duration,
};

use serde::Deserialize;
use serde::Serialize;
use sov_rollup_interface::traits::CanonicalHash;

use demo_stf::app::{get_rpc_methods, DemoBatchReceipt, DemoTxReceipt, NativeAppRunner};
use jupiter::{
    da_service::{CelestiaService, DaServiceConfig},
    types::FilteredCelestiaBlock,
    verifier::RollupParams,
};
use risc0_adapter::host::Risc0Host;
use sha2::digest::typenum::private::PrivateAnd;
use sov_db::ledger_db::{LedgerDB, SlotCommit};
use sov_modules_api::RpcRunner;
use sov_rollup_interface::{
    services::da::{DaService, SlotData},
    stf::{BatchReceipt, StateTransitionFunction, StateTransitionRunner, TransactionReceipt},
    traits::BlockHeaderTrait,
};
use sov_state::{config::Config, Storage};
use tracing::{debug, info, Level};

use demo_stf::runner_config::Config as RunnerConfig;
use demo_stf::runner_config::{from_toml_path, StorageConfig};

use crate::{
    config::{RollupConfig, RpcConfig},
    get_genesis_config, initialize_ledger, ledger_rpc, start_rpc_server, ROLLUP_NAMESPACE,
};

use curl::easy::{Easy2, Handler, List, WriteError};
use tokio::{sync::oneshot, time::sleep};

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
            events: vec![],
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
            ledger_rpc::get_ledger_rpc::<DemoBatchReceipt, DemoTxReceipt>(ledger_db.clone());

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
        let expected = r#"{"jsonrpc":"2.0","result":[{"hash":"0x329c58b0a9b08973bed32452c2cefa0ab567146505711337c955b24cf41c6e99","event_range":{"start":1,"end":1},"body":[116,120,49,32,98,111,100,121],"custom_receipt":"Reverted"}],"id":1}"#;
        query_test_helper(data, expected);

        let data = r#"{"jsonrpc":"2.0","method":"ledger_getTransactions","params":[[{ "batch_id": 1, "offset": 1}]],"id":1}"#
            .as_bytes();
        let expected = r#"{"jsonrpc":"2.0","result":[{"hash":"0xae87c50974dc43f4f70e84cb27e4630e4e47f782cff1e3d484310d82cea9acf6","event_range":{"start":1,"end":1},"body":[116,120,50,32,98,111,100,121],"custom_receipt":"Successful"}],"id":1}"#;
        query_test_helper(data, expected);
        
        
        let data = r#"{"jsonrpc":"2.0","method":"ledger_getBatches","params":[[2], "Standard"],"id":1}"#
            .as_bytes();
        let expected = r#"{"jsonrpc":"2.0","error":{"code":-32000,"message":"io error: unexpected end of file"},"id":1}"#;
        query_test_helper(data, expected);

        let data = r#"{"jsonrpc":"2.0","method":"ledger_getBatches","params":[[0], "Standard"],"id":2}"#
            .as_bytes();
        let expected = r#"{"jsonrpc":"2.0","result":[null],"id":2}"#;
        query_test_helper(data, expected);


        let data = r#"{"jsonrpc":"2.0","method":"ledger_getEvents","params":[1],"id":1}"#.as_bytes();
        let expected = r#"{"jsonrpc":"2.0","result":[null],"id":1}"#;
        query_test_helper(data, expected);

        tx_end.send("drop server").unwrap();
    });

}
