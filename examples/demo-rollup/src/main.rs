use std::env;

use anyhow::Context;
use demo_stf::app::{App, DefaultContext};
use demo_stf::runtime::get_rpc_methods;
use jupiter::da_service::CelestiaService;
use jupiter::verifier::RollupParams;
#[cfg(feature = "experimental")]
use sov_demo_rollup::register_rpc::register_ethereum;
use sov_demo_rollup::register_rpc::{register_ledger, register_sequencer};
use sov_demo_rollup::{get_genesis_config, initialize_ledger, ROLLUP_NAMESPACE};
use sov_rollup_interface::services::da::DaService;
use sov_state::storage::Storage;
use sov_stf_runner::{from_toml_path, RollupConfig, StateTransitionRunner};
use tracing::{debug, Level};

#[cfg(test)]
mod test_rpc;

/// Main demo runner. Initialize a DA chain, and starts a demo-rollup using the config provided
/// (or a default config if not provided). Then start checking the blocks sent to the DA layer in
/// the main event loop.

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let rollup_config_path = env::args()
        .nth(1)
        .unwrap_or_else(|| "rollup_config.toml".to_string());

    debug!("Starting demo rollup with config {}", rollup_config_path);
    let rollup_config: RollupConfig =
        from_toml_path(&rollup_config_path).context("Failed to read rollup configuration")?;

    // Initializing logging
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber)
        .map_err(|_err| eprintln!("Unable to set global default subscriber"))
        .expect("Cannot fail to set subscriber");

    let ledger_db = initialize_ledger(&rollup_config.storage.path);

    let da_service = CelestiaService::new(
        rollup_config.da.clone(),
        RollupParams {
            namespace: ROLLUP_NAMESPACE,
        },
    )
    .await;

    let mut app = App::new(rollup_config.storage.clone());

    let storage = app.get_storage();
    let mut methods = get_rpc_methods::<DefaultContext>(storage);

    // register rpc methods
    {
        register_ledger(ledger_db.clone(), &mut methods)?;
        register_sequencer(da_service.clone(), &mut app, &mut methods)?;
        #[cfg(feature = "experimental")]
        register_ethereum(da_service.clone(), &mut methods)?;
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
