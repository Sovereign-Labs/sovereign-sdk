use std::env;

use anyhow::Context;
use borsh::{BorshDeserialize, BorshSerialize};
use demo_stf::app::{App, DefaultContext};
use demo_stf::runtime::get_rpc_methods;
use jupiter::da_service::CelestiaService;
#[cfg(feature = "experimental")]
use jupiter::da_service::DaServiceConfig;
use jupiter::verifier::{CelestiaSpec, ChainValidityCondition, RollupParams};
use risc0_adapter::host::Risc0Verifier;
use sov_demo_rollup::register_rpc::{register_ledger, register_sequencer};
use sov_demo_rollup::{get_genesis_config, initialize_ledger, ROLLUP_NAMESPACE};
#[cfg(feature = "experimental")]
use sov_ethereum::get_ethereum_rpc;
use sov_rollup_interface::services::da::DaService;
use sov_rollup_interface::zk::ValidityConditionChecker;
use sov_state::storage::Storage;
use sov_stf_runner::{from_toml_path, RollupConfig, StateTransitionRunner};
use tracing::{debug, Level};

#[cfg(test)]
mod test_rpc;

/// Main demo runner. Initialize a DA chain, and starts a demo-rollup using the config provided
/// (or a default config if not provided). Then start checking the blocks sent to the DA layer in
/// the main event loop.
#[derive(Debug, BorshSerialize, BorshDeserialize)]
pub struct CelestiaChainChecker {
    current_block_hash: [u8; 32],
}

impl ValidityConditionChecker<ChainValidityCondition> for CelestiaChainChecker {
    type Error = anyhow::Error;

    fn check(&mut self, condition: &ChainValidityCondition) -> Result<(), anyhow::Error> {
        anyhow::ensure!(
            condition.block_hash == self.current_block_hash,
            "Invalid block hash"
        );
        Ok(())
    }
}

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

    let ledger_db = initialize_ledger(&rollup_config.runner.storage.path);

    let da_service = CelestiaService::new(
        rollup_config.da.clone(),
        RollupParams {
            namespace: ROLLUP_NAMESPACE,
        },
    )
    .await;

    let mut app: App<Risc0Verifier, CelestiaSpec> = App::new(rollup_config.runner.storage.clone());

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
