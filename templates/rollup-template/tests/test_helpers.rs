use std::net::SocketAddr;
use std::path::Path;

use rollup_template::rollup::Rollup;
use sov_rollup_interface::mocks::{
    MockAddress, MockDaConfig, MockDaService, MOCK_SEQUENCER_DA_ADDRESS,
};
use sov_rollup_interface::zk::ZkvmHost;
use sov_stf_runner::{RollupConfig, RpcConfig, RunnerConfig, StorageConfig};
use template_stf::{get_genesis_config, GenesisPaths};
use tokio::sync::oneshot;

pub async fn start_rollup<Vm: ZkvmHost, P: AsRef<Path>>(
    rpc_reporting_channel: oneshot::Sender<SocketAddr>,
    genesis_paths: &GenesisPaths<P>,
) {
    let temp_dir = tempfile::tempdir().unwrap();
    let temp_path = temp_dir.path();

    let sequencer_da_address = MockAddress::from(MOCK_SEQUENCER_DA_ADDRESS);

    let genesis_config = get_genesis_config(sequencer_da_address, genesis_paths);

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
            sender_address: genesis_config.sequencer_registry.seq_da_address,
        },
    };

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
