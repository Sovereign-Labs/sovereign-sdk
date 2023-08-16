use std::env;

use anyhow::Context;
use demo_stf::app::App;
use demo_stf::runtime::get_rpc_methods;
use risc0_adapter::host::Risc0Verifier;
use sov_demo_rollup::register_rpc::{register_ledger, register_sequencer};
use sov_demo_rollup::{get_genesis_config, initialize_ledger};
use sov_modules_api::default_context::DefaultContext;
use sov_rollup_interface::mocks::{MockAddress, MockBlob, MockDaService, MockValidityCond};
use sov_state::storage::Storage;
use sov_stf_runner::{from_toml_path, RollupConfig, StateTransitionRunner};
use tracing::debug;

#[tokio::test]
async fn rollup_test() -> Result<(), anyhow::Error> {
    let rollup_config_path = env::args()
        .nth(1)
        .unwrap_or_else(|| "rollup_config.toml".to_string());

    debug!("Starting demo rollup with config {}", rollup_config_path);
    let rollup_config: RollupConfig =
        from_toml_path(&rollup_config_path).context("Failed to read rollup configuration")?;

    let ledger_db = initialize_ledger(&rollup_config.runner.storage.path);

    let da_service = MockDaService::default();
    let mut app: App<Risc0Verifier, MockValidityCond, MockBlob<MockAddress>> =
        App::new(rollup_config.runner.storage.clone());

    let storage = app.get_storage();
    let mut methods = get_rpc_methods::<DefaultContext>(storage);

    // register rpc methods
    {
        register_ledger(ledger_db.clone(), &mut methods)?;
        register_sequencer(da_service.clone(), &mut app, &mut methods)?;
        #[cfg(feature = "experimental")]
        register_ethereum(rollup_config.da.clone(), &mut methods)?;
    }

    let storage = app.get_storage();
    let genesis_config = get_genesis_config();

    let mut runner = StateTransitionRunner::new(
        rollup_config,
        da_service,
        ledger_db,
        app.stf,
        storage.is_empty(),
        genesis_config,
    )?;

    runner.start_rpc_server(methods).await;
    runner.run().await?;

    Ok(())
}
