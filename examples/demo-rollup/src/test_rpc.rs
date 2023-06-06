use std::{net::SocketAddr, path::PathBuf, time::Duration};

use demo_stf::app::{DemoBatchReceipt, DemoTxReceipt, NativeAppRunner};
use jupiter::{
    da_service::{CelestiaService, DaServiceConfig},
    verifier::RollupParams,
};
use risc0_adapter::host::Risc0Host;
use sov_rollup_interface::{services::da::DaService, stf::StateTransitionRunner};
use sov_state::config::Config;
use tokio::{process::Command, time::sleep};
use tracing::Level;

use demo_stf::runner_config::Config as RunnerConfig;
use demo_stf::runner_config::{from_toml_path, StorageConfig};

use crate::{
    config::{RollupConfig, RpcConfig},
    initialize_ledger, ledger_rpc, start_rpc_server, ROLLUP_NAMESPACE,
};

use curl::easy::{Easy2, Handler, List, WriteError};

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

#[tokio::test]
async fn readme_test_rpc() {
    //celestia light start --core.ip https://limani.celestia-devops.dev --p2p.network arabica --gateway --rpc.port 11111
    let mut celestia_task = Command::new("celestia")
        .arg("light")
        .arg("start")
        .arg("--core.ip")
        .arg("https://limani.celestia-devops.dev")
        .arg("--p2p.network")
        .arg("arabica")
        .arg("--gateway")
        .arg("--rpc.port")
        .arg("11111")
        .spawn()
        .expect("failed to execute process");

    sleep(Duration::from_secs(10)).await;

    let mut rpc_server = Command::new("cargo")
        .arg("run")
        .spawn()
        .expect("failed to execute process");

    sleep(Duration::from_secs(10)).await;

    let data = r#"{"jsonrpc":"2.0","method":"ledger_getSlots","params":[[7], "Compact"],"id":1}"#
        .as_bytes();
    let expected = r#"{"jsonrpc":"2.0","result":[{"number":7,"hash":"0x4083ae3bf35acdcfe6e6d78841bbab2b28b8e051b1e3f89ea01a9bf740dd4d67","batch_range":{"start":2,"end":2}}],"id":1}"#;
    query_test_helper(data, expected);

    let data = r#"{"jsonrpc":"2.0","method":"ledger_getBatches","params":[["0xf784a42555ed652ed045cc8675f5bc11750f1c7fb0fbc8d6a04470a88c7e1b6c"]],"id":1}"#
        .as_bytes();
    let expected = r#"{"jsonrpc":"2.0","result":[{"hash":"0xf784a42555ed652ed045cc8675f5bc11750f1c7fb0fbc8d6a04470a88c7e1b6c","tx_range":{"start":1,"end":1},"txs":[],"custom_receipt":{"Slashed":"InvalidTransactionEncoding"}}],"id":1}"#;
    query_test_helper(data, expected);

    let data = r#"{"jsonrpc":"2.0","method":"ledger_getTransactions","params":[[{ "batch_id": 1, "offset": 0}]],"id":1}"#
        .as_bytes();
    let expected = r#"{"jsonrpc":"2.0","result":[null],"id":1}"#;
    query_test_helper(data, expected);

    let data = r#"{"jsonrpc":"2.0","method":"ledger_getEvents","params":[1],"id":1}"#.as_bytes();
    let expected = r#"{"jsonrpc":"2.0","result":[null],"id":1}"#;
    query_test_helper(data, expected);

    rpc_server.kill().await.unwrap();
    println!("Killed the rpc server");

    celestia_task.kill().await.unwrap();
    println!("Killed the celestia server");
}
