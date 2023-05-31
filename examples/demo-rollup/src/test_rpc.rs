use std::{net::SocketAddr, path::PathBuf, thread::sleep, time::Duration};

use demo_stf::app::{DemoBatchReceipt, DemoTxReceipt, NativeAppRunner};
use jupiter::{
    da_service::{CelestiaService, DaServiceConfig},
    verifier::RollupParams,
};
use risc0_adapter::host::Risc0Host;
use sov_rollup_interface::{services::da::DaService, stf::StateTransitionRunner};
use sov_state::config::Config;
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

#[tokio::test]
async fn simple_test_rpc() {
    let config: RollupConfig = RollupConfig {
        start_height: 31337,
        da: DaServiceConfig {
            celestia_rpc_auth_token: "SECRET_RPC_TOKEN".to_string(),
            celestia_rpc_address: "http://localhost:11111/".into(),
            max_celestia_response_body_size: 980,
        },
        runner: RunnerConfig {
            storage: StorageConfig {
                path: PathBuf::from("/tmp"),
            },
        },
        rpc_config: RpcConfig {
            bind_host: "127.0.0.1".to_string(),
            bind_port: 12345,
        },
    };

    let rpc_config = config.rpc_config;
    let address = SocketAddr::new(rpc_config.bind_host.parse().unwrap(), rpc_config.bind_port);

    // Initializing logging
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber)
        .map_err(|_err| eprintln!("Unable to set global default subscriber"))
        .expect("Cannot fail to set subscriber");

    // Initialize the ledger database, which stores blocks, transactions, events, etc.
    let ledger_db = initialize_ledger(&config.runner.storage.path);

    let ledger_rpc_module =
        ledger_rpc::get_ledger_rpc::<DemoBatchReceipt, DemoTxReceipt>(ledger_db.clone());

    let _handle = tokio::spawn(async move {
        start_rpc_server(ledger_rpc_module, address).await;
    });

    // Initialize the Celestia service using the DaService interface
    let da_service = CelestiaService::new(
        config.da.clone(),
        RollupParams {
            namespace: ROLLUP_NAMESPACE,
        },
    );

    let mut headers = List::new();
    headers.append("Content-Type: application/json").unwrap();

    let data = "{\"jsonrpc\":\"2.0\",\"method\":\"ledger_head\",\"params\":[],\"id\":1}".as_bytes();

    let mut easy = Easy2::new(Collector(Vec::new()));
    easy.http_headers(headers).unwrap();
    easy.post_fields_copy(data).unwrap();
    easy.post(true).unwrap();

    easy.url("http://127.0.0.1:12345").unwrap();
    easy.perform().unwrap();

    assert_eq!(easy.response_code().unwrap(), 200);
    let contents = easy.get_ref();
    println!("{}", String::from_utf8_lossy(&contents.0));
}
