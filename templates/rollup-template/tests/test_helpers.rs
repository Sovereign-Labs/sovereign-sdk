use std::net::SocketAddr;

use rollup_template::{rollup::Rollup, runtime::GenesisConfig};
use sov_modules_api::default_context::DefaultContext;
use sov_rollup_interface::{
    mocks::{MockDaConfig, MockDaService, MockDaSpec},
    zk::ZkvmHost,
};
use sov_stf_runner::{RollupConfig, RpcConfig, RunnerConfig, StorageConfig};
use tokio::sync::oneshot;

pub async fn start_rollup<Vm: ZkvmHost>(rpc_reporting_channel: oneshot::Sender<SocketAddr>) {
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
    let genesis_config = serde_json::from_str::<GenesisConfig<DefaultContext, MockDaSpec>>(
        include_str!("test_genesis.json"),
    )
    .unwrap();
    let rollup = Rollup::<Vm, _>::new(
        MockDaService::new(genesis_config.sequencer_registry.seq_da_address),
        genesis_config,
        rollup_config,
        None,
    )
    .unwrap();

    rollup
        .run_and_report_rpc_port(Some(rpc_reporting_channel))
        .await
        .unwrap();

    // Close the tempdir explicitly to ensure that rustc doesn't see that it's unused and drop it unexpectedly
    temp_dir.close().unwrap();
}
