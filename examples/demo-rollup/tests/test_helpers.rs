use std::fs::remove_dir_all;
use std::net::SocketAddr;
use std::path::PathBuf;

use demo_stf::app::App;
use ethers_contract::BaseContract;
use ethers_core::abi::Abi;
use ethers_core::types::Bytes;
use revm::primitives::{ExecutionResult, Output};
use risc0_adapter::host::Risc0Verifier;
use sov_demo_rollup::{get_genesis_config, initialize_ledger, Rollup};
use sov_rollup_interface::mocks::{MockAddress, MockDaService};
use sov_stf_runner::{RollupConfig, RpcConfig, RunnerConfig, StorageConfig};
use tokio::sync::oneshot;

#[allow(dead_code)]
pub(crate) fn output(result: ExecutionResult) -> bytes::Bytes {
    match result {
        ExecutionResult::Success { output, .. } => match output {
            Output::Call(out) => out,
            Output::Create(out, _) => out,
        },
        _ => panic!("Expected successful ExecutionResult"),
    }
}

#[allow(dead_code)]
fn test_data_path() -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests");
    path.push("test_data");
    path
}

#[allow(dead_code)]
fn make_contract_from_abi(path: PathBuf) -> BaseContract {
    let abi_json = std::fs::read_to_string(path).unwrap();
    let abi: Abi = serde_json::from_str(&abi_json).unwrap();
    BaseContract::from(abi)
}

fn create_mock_da_rollup(rollup_config: RollupConfig<()>) -> Rollup<Risc0Verifier, MockDaService> {
    let _ = remove_dir_all(&rollup_config.storage.path);
    let ledger_db = initialize_ledger(rollup_config.storage.path.clone());
    let sequencer_da_address = MockAddress { addr: [99; 32] };
    let da_service = MockDaService::new(sequencer_da_address);

    let app = App::new(rollup_config.storage);

    let genesis_config = get_genesis_config(sequencer_da_address);

    Rollup {
        app,
        da_service,
        ledger_db,
        runner_config: rollup_config.runner,
        genesis_config,
    }
}

pub async fn start_rollup(rpc_reporting_channel: oneshot::Sender<SocketAddr>) {
    let mut mock_path = PathBuf::from("tests");
    mock_path.push("test_data");
    mock_path.push("tmp");
    mock_path.push("mocks");

    let rollup_config = RollupConfig {
        storage: StorageConfig { path: mock_path },
        runner: RunnerConfig {
            start_height: 0,
            rpc_config: RpcConfig {
                bind_host: "127.0.0.1".into(),
                bind_port: 0,
            },
        },
        da: (),
    };
    let rollup = create_mock_da_rollup(rollup_config);
    rollup
        .run_and_report_rpc_port(Some(rpc_reporting_channel))
        .await
        .unwrap();
}

#[allow(dead_code)]
pub(crate) struct SimpleStorageContract {
    bytecode: Bytes,
    base_contract: BaseContract,
}

impl SimpleStorageContract {
    #[allow(dead_code)]
    pub(crate) fn new() -> Self {
        let contract_data = {
            let mut path = test_data_path();
            path.push("SimpleStorage.bin");

            let contract_data = std::fs::read_to_string(path).unwrap();
            hex::decode(contract_data).unwrap()
        };

        let contract = {
            let mut path = test_data_path();
            path.push("SimpleStorage.abi");

            make_contract_from_abi(path)
        };

        Self {
            bytecode: Bytes::from(contract_data),
            base_contract: contract,
        }
    }

    #[allow(dead_code)]
    pub(crate) fn byte_code(&self) -> Bytes {
        self.bytecode.clone()
    }

    #[allow(dead_code)]
    pub(crate) fn set_call_data(&self, set_arg: u32) -> Bytes {
        let set_arg = ethereum_types::U256::from(set_arg);
        self.base_contract.encode("set", set_arg).unwrap()
    }

    #[allow(dead_code)]
    pub(crate) fn get_call_data(&self) -> Bytes {
        self.base_contract.encode("get", ()).unwrap()
    }
}
