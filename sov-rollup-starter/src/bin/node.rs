//! This binary runs the rollup full node.

use anyhow::Context;
use clap::Parser;
use sov_modules_rollup_template::{Rollup, RollupProverConfig, RollupTemplate};
use sov_rollup_interface::mocks::MockDaConfig;
use sov_rollup_starter::rollup::StarterRollup;
use sov_stf_runner::{from_toml_path, RollupConfig};
use std::path::PathBuf;
use stf_starter::genesis_config::GenesisPaths;
use tracing::info;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{fmt, EnvFilter};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// The path to the rollup config.
    #[arg(long, default_value = "rollup_config.toml")]
    rollup_config_path: String,

    /// The path to the genesis config.
    #[arg(long, default_value = "test-data/genesis/")]
    genesis_paths: String,
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    // Initializing logging
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env())
        .init();

    let args = Args::parse();
    let rollup_config_path = args.rollup_config_path.as_str();

    let genesis_paths = args.genesis_paths.as_str();

    let rollup = new_rollup_with_mock_da(
        &GenesisPaths::from_dir(genesis_paths),
        rollup_config_path,
        Some(RollupProverConfig::Execute),
    )
    .await?;
    rollup.run().await
}

async fn new_rollup_with_mock_da(
    genesis_paths: &GenesisPaths<PathBuf>,
    rollup_config_path: &str,
    prover_config: Option<RollupProverConfig>,
) -> Result<Rollup<StarterRollup>, anyhow::Error> {
    info!("Reading rollup config from {rollup_config_path:?}");

    let rollup_config: RollupConfig<MockDaConfig> =
        from_toml_path(rollup_config_path).context("Failed to read rollup configuration")?;

    let starter_rollup = StarterRollup {};
    starter_rollup
        .create_new_rollup(genesis_paths, rollup_config, prover_config)
        .await
}
