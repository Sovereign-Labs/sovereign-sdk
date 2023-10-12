use std::net::SocketAddr;
use std::path::Path;

use demo_stf::genesis_config::GenesisPaths;
use sov_demo_rollup::{new_rollup_with_mock_da_from_config, DemoProverConfig};
use sov_rollup_interface::mocks::{MockAddress, MockDaConfig};
use sov_rollup_interface::zk::ZkvmHost;
use sov_stf_runner::{RollupConfig, RpcConfig, RunnerConfig, StorageConfig};
use tokio::sync::oneshot;

pub async fn start_rollup<Vm: ZkvmHost, P: AsRef<Path>>(
    rpc_reporting_channel: oneshot::Sender<SocketAddr>,
    prover: Option<(Vm, DemoProverConfig)>,
    genesis_paths: &GenesisPaths<P>,
) {
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
        da: MockDaConfig {
            sender_address: MockAddress { addr: [0; 32] },
        },
    };

    let rollup = new_rollup_with_mock_da_from_config(rollup_config, prover, genesis_paths)
        .expect("Rollup config is valid");
    rollup
        .run_and_report_rpc_port(Some(rpc_reporting_channel))
        .await
        .unwrap();

    // Close the tempdir explicitly to ensure that rustc doesn't see that it's unused and drop it unexpectedly
    temp_dir.close().unwrap();
}
