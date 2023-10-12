//! This binary runs the rollup full node.

use std::env;
use std::path::PathBuf;

use anyhow::Context;
use rollup_template::da::{start_da_service, DaConfig};
use rollup_template::rollup::Rollup;
use sov_risc0_adapter::host::Risc0Host;
use sov_rollup_interface::mocks::{MockAddress, MOCK_SEQUENCER_DA_ADDRESS};
use sov_stf_runner::{from_toml_path, RollupConfig};
use template_stf::{get_genesis_config, GenesisPaths};
use tracing::info;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    // Initialize a logger for the demo
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::builder()
                .with_default_directive(LevelFilter::INFO.into()) // If no logging config is set. default to `info` level logs
                .from_env_lossy(), // Parse the log level from the RUST_LOG env var if set
        ) // Try to override logging config from RUST_LOG env var
        .finish();
    tracing::subscriber::set_global_default(subscriber)
        .context("Unable to set global default subscriber")?;

    // Read the rollup config from a file
    let rollup_config_path = env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("rollup_config.toml"));
    info!("Reading rollup config from {rollup_config_path:?}");

    let rollup_config: RollupConfig<DaConfig> =
        from_toml_path(rollup_config_path).context("Failed to read rollup configuration")?;
    info!("Initializing DA service");
    let da_service = start_da_service(&rollup_config).await;

    let sequencer_da_address = MockAddress::from(MOCK_SEQUENCER_DA_ADDRESS);
    let genesis_paths = GenesisPaths::from_dir("../../test-data/genesis/");
    let genesis_config = get_genesis_config(sequencer_da_address, &genesis_paths);

    // Start rollup
    let rollup: Rollup<Risc0Host, _> =
        Rollup::new(da_service, genesis_config, rollup_config, None)?;

    rollup.run().await?;

    Ok(())
}
