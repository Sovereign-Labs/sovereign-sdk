use std::net::SocketAddr;

use sov_demo_rollup::new_rollup_with_mock_da_from_config;
#[cfg(feature = "experimental")]
use sov_rollup_interface::mocks::MockDaConfig;
use sov_stf_runner::{RollupConfig, RpcConfig, RunnerConfig, StorageConfig};
use tokio::sync::oneshot;

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
        da: MockDaConfig {},
    };
    let rollup =
        new_rollup_with_mock_da_from_config(rollup_config).expect("Rollup config is valid");
    rollup
        .run_and_report_rpc_port(Some(rpc_reporting_channel))
        .await
        .unwrap();

    // Close the tempdir explicitly to ensure that rustc doesn't see that it's unused and drop it unexpectedly
    temp_dir.close().unwrap();
}
