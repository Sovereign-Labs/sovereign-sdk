use std::{
    collections::hash_map::DefaultHasher,
    hash::{self, Hash, Hasher},
    net::SocketAddr,
    path::PathBuf,
    time::Duration,
};

use demo_stf::app::{get_rpc_methods, DemoBatchReceipt, DemoTxReceipt, NativeAppRunner};
use jupiter::{
    da_service::{CelestiaService, DaServiceConfig},
    verifier::RollupParams,
};
use risc0_adapter::host::Risc0Host;
use sha2::digest::typenum::private::PrivateAnd;
use sov_db::ledger_db::{LedgerDB, SlotCommit};
use sov_modules_api::RpcRunner;
use sov_rollup_interface::{
    services::da::DaService,
    stf::{BatchReceipt, StateTransitionFunction, StateTransitionRunner},
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

fn populate_ledger(ledger_db: LedgerDB) {
    let mut hasher = DefaultHasher::new();
    1.hash(&mut hasher);

    let slot: SlotCommit<&str> = SlotCommit {
        slot_data: "Hello word",
        batch_receipts: vec![BatchReceipt {
            batch_hash: hasher.finish(),
        }],
    };
    ledger_db.commit_slot(slot);
}

#[tokio::test]
async fn simple_test_rpc() {
    let (tx, rx) = oneshot::channel();
    let rpc_config = RpcConfig {
        bind_host: "127.0.0.1".to_string(),
        bind_port: 12345,
    };

    let address = SocketAddr::new(rpc_config.bind_host.parse().unwrap(), rpc_config.bind_port);

    // Initialize the ledger database, which stores blocks, transactions, events, etc.
    let ledger_db = LedgerDB::temporary();

    let ledger_rpc_module =
        ledger_rpc::get_ledger_rpc::<DemoBatchReceipt, DemoTxReceipt>(ledger_db.clone());

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_io()
        .enable_time()
        .build()
        .unwrap();

    let _handle = rt.spawn(async move {
        let server = jsonrpsee::server::ServerBuilder::default()
            .build([address].as_ref())
            .await
            .unwrap();
        let _server_handle = server.start(ledger_rpc_module).unwrap();
        tx.send("server started").unwrap();
        futures::future::pending::<()>().await;
    });

    // Wait for the server to start
    rx.await.unwrap();

    let data = r#"{"jsonrpc":"2.0","method":"ledger_getSlots","params":[[7], "Compact"],"id":1}"#
        .as_bytes();
    let expected = r#"{"jsonrpc":"2.0","result":[{"number":7,"hash":"0x4083ae3bf35acdcfe6e6d78841bbab2b28b8e051b1e3f89ea01a9bf740dd4d67","batch_range":{"start":2,"end":2}}],"id":1}"#;
    query_test_helper(data, expected);

    println!("Sent request");
}
