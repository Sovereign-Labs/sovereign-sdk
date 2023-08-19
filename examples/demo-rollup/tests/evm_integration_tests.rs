use core::panic;
use std::path::Path;

use demo_stf::app::App;
use risc0_adapter::host::Risc0Verifier;
use sov_demo_rollup::{initialize_ledger, Rollup};
use sov_rollup_interface::mocks::MockDaService;
use sov_stf_runner::{RollupConfig, RpcConfig, RunnerConfig, StorageConfig};

fn create_mock_da_rollup(rollup_config: RollupConfig<()>) -> Rollup<Risc0Verifier, MockDaService> {
    let ledger_db = initialize_ledger(rollup_config.storage.path.clone());
    let da_service = MockDaService::default();

    let app = App::new(rollup_config.storage);

    Rollup {
        app,
        da_service,
        ledger_db,
        runner_config: rollup_config.runner,
    }
}

#[tokio::test]
async fn tx_tests() -> Result<(), anyhow::Error> {
    let rollup_config = RollupConfig {
        storage: StorageConfig {
            path: "/tmp".into(),
        },
        runner: RunnerConfig {
            start_height: 0,
            rpc_config: RpcConfig {
                bind_host: "127.0.0.1".into(),
                bind_port: 12345,
            },
        },
        da: (),
    };

    let rollup = create_mock_da_rollup(rollup_config);
    rollup.run().await
}
