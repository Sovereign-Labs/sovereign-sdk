use std::str::FromStr;

use anyhow::Context as _;
use clap::Parser;
use demo_stf::genesis_config::GenesisPaths;
use sov_demo_rollup::{CelestiaDemoRollup, MockDemoRollup};
use sov_mock_da::MockDaConfig;
use sov_modules_rollup_template::{Rollup, RollupProverConfig, RollupTemplate};
use sov_stf_runner::{from_toml_path, RollupConfig};
use tracing::log::debug;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{fmt, EnvFilter};

#[cfg(test)]
mod test_rpc;

/// Main demo runner. Initialize a DA chain, and starts a demo-rollup using the config provided
/// (or a default config if not provided). Then start checking the blocks sent to the DA layer in
/// the main event loop.

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// The data layer type.
    #[arg(long, default_value = "celestia")]
    da_layer: String,

    /// The path to the rollup config.
    #[arg(long, default_value = "rollup_config.toml")]
    rollup_config_path: String,
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    // Initializing logging
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_str("debug,hyper=info,guest_execution=info").unwrap())
        .init();

    let args = Args::parse();
    let rollup_config_path = args.rollup_config_path.as_str();

    match args.da_layer.as_str() {
        "mock" => {
            let rollup = new_rollup_with_mock_da(
                &GenesisPaths::from_dir("../test-data/genesis/integration-tests"),
                rollup_config_path,
                Some(RollupProverConfig::Execute),
            )
            .await?;
            rollup.run().await
        }
        "celestia" => {
            let rollup = new_rollup_with_celestia_da(
                &GenesisPaths::from_dir("../test-data/genesis/demo-tests"),
                rollup_config_path,
                Some(RollupProverConfig::Execute),
            )
            .await?;
            rollup.run().await
        }
        da => panic!("DA Layer not supported: {}", da),
    }
}

async fn new_rollup_with_celestia_da(
    genesis_paths: &GenesisPaths,
    rollup_config_path: &str,
    prover_config: Option<RollupProverConfig>,
) -> Result<Rollup<CelestiaDemoRollup>, anyhow::Error> {
    debug!(
        "Starting celestia rollup with config {}",
        rollup_config_path
    );

    let rollup_config: RollupConfig<sov_celestia_adapter::CelestiaConfig> =
        from_toml_path(rollup_config_path).context("Failed to read rollup configuration")?;

    let mock_rollup = CelestiaDemoRollup {};
    mock_rollup
        .create_new_rollup(genesis_paths, rollup_config, prover_config)
        .await
}

async fn new_rollup_with_mock_da(
    genesis_paths: &GenesisPaths,
    rollup_config_path: &str,
    prover_config: Option<RollupProverConfig>,
) -> Result<Rollup<MockDemoRollup>, anyhow::Error> {
    debug!("Starting mock rollup with config {}", rollup_config_path);

    let rollup_config: RollupConfig<MockDaConfig> =
        from_toml_path(rollup_config_path).context("Failed to read rollup configuration")?;

    let mock_rollup = MockDemoRollup {};
    mock_rollup
        .create_new_rollup(genesis_paths, rollup_config, prover_config)
        .await
}
