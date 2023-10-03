//! This binary runs the rollup full node.

use std::env;
use std::path::PathBuf;

use anyhow::Context;
use rollup_template::da::{start_da_service, DaConfig};
use rollup_template::rollup::Rollup;
use sov_risc0_adapter::host::Risc0Host;
use sov_stf_runner::{from_toml_path, RollupConfig};
use tracing::{debug, info, trace};
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    // Initialize a logger for the demo
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::builder()
                .with_default_directive(LevelFilter::DEBUG.into()) // If no logging config is set. default to `info` level logs
                .from_env_lossy(), // Parse the log level from the RUST_LOG env var if set
        ) // Try to override logging config from RUST_LOG env var
        .finish();
    tracing::subscriber::set_global_default(subscriber)
        .context("Unable to set global default subscriber")?;
    let rollup_config_path = env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("rollup_config.toml"));
    info!("Reading rollup config from {rollup_config_path:?}");
    // Read the rollup config from a file
    let rollup_config: RollupConfig<DaConfig> =
        from_toml_path(rollup_config_path).context("Failed to read rollup configuration")?;
    info!("Initializing DA service");
    let da_service = start_da_service(&rollup_config).await;

    // Load genesis data from file
    let genesis_path = env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("genesis.json"));
    info!("Reading genesis configuration from {genesis_path:?}");
    let genesis_config =
        std::fs::read_to_string(genesis_path).context("Failed to read genesis configuration")?;
    debug!("Genesis config size: {} bytes", genesis_config.len());
    trace!("Genesis config: {}", &genesis_config);
    let genesis_config = serde_json::from_str(&genesis_config)?;

    let rollup: Rollup<Risc0Host, _> =
        Rollup::new(da_service, genesis_config, rollup_config, None)?;

    rollup.run().await?;

    Ok(())
}
