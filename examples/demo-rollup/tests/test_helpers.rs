use std::fs::remove_dir_all;
use std::net::SocketAddr;

use demo_stf::app::App;
use risc0_adapter::host::Risc0Verifier;
use sov_demo_rollup::{get_genesis_config, initialize_ledger, Rollup};
use sov_rollup_interface::mocks::{MockAddress, MockDaService};
use sov_stf_runner::{RollupConfig, RpcConfig, RunnerConfig, StorageConfig};
use tokio::sync::oneshot;

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
    let temp_dir = tempfile::tempdir().unwrap();
    let temp_path = temp_dir.path();

    let rollup_config = RollupConfig {
        storage: StorageConfig {
            path: temp_path.to_path_buf(),
        },
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

    // Close the tempdir explicitly to ensure that rustc doesn't see that it's unused and drop it unexpectedly
    temp_dir.close().unwrap();
}
