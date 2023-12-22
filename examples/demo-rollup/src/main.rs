use anyhow::Context as _;
use clap::Parser;
use demo_stf::genesis_config::GenesisPaths;
use sov_demo_rollup::{initialize_logging, CelestiaDemoRollup, MockDemoRollup};
use sov_mock_da::MockDaConfig;
use sov_modules_rollup_blueprint::{Rollup, RollupBlueprint};
use sov_modules_stf_blueprint::kernels::basic::{
    BasicKernelGenesisConfig, BasicKernelGenesisPaths,
};
use sov_stf_runner::{from_toml_path, RollupConfig, RollupProverConfig};
use tracing::log::debug;

#[cfg(test)]
mod test_rpc;

/// Main demo runner. Initializes a DA chain, and starts a demo-rollup using the provided.
/// If you're trying to sign or submit transactions to the rollup, the `sov-cli` binary
/// is the one you want. You can run it `cargo run --bin sov-cli`.

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// The data layer type.
    #[arg(long, default_value = "mock")]
    da_layer: SupportedDaLayer,

    /// The path to the rollup config.
    #[arg(long, default_value = "mock_rollup_config.toml")]
    rollup_config_path: String,
}

#[derive(clap::ValueEnum, Clone, Debug)]
enum SupportedDaLayer {
    Celestia,
    Mock,
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    initialize_logging();

    let args = Args::parse();
    let rollup_config_path = args.rollup_config_path.as_str();

    match args.da_layer {
        SupportedDaLayer::Mock => {
            let rollup = new_rollup_with_mock_da(
                &GenesisPaths::from_dir("../test-data/genesis/demo-tests/mock"),
                &BasicKernelGenesisPaths {
                    chain_state: "../test-data/genesis/demo-tests/mock/chain_state.json".into(),
                },
                rollup_config_path,
                RollupProverConfig::Execute,
            )
            .await?;
            rollup.run().await
        }
        SupportedDaLayer::Celestia => {
            let rollup = new_rollup_with_celestia_da(
                &GenesisPaths::from_dir("../test-data/genesis/demo-tests/celestia"),
                &BasicKernelGenesisPaths {
                    chain_state: "../test-data/genesis/demo-tests/celestia/chain_state.json".into(),
                },
                rollup_config_path,
                RollupProverConfig::Execute,
            )
            .await?;
            rollup.run().await
        }
    }
}

async fn new_rollup_with_celestia_da(
    rt_genesis_paths: &GenesisPaths,
    kernel_genesis_paths: &BasicKernelGenesisPaths,
    rollup_config_path: &str,
    prover_config: RollupProverConfig,
) -> Result<Rollup<CelestiaDemoRollup>, anyhow::Error> {
    debug!(
        "Starting celestia rollup with config {}",
        rollup_config_path
    );

    let rollup_config: RollupConfig<sov_celestia_adapter::CelestiaConfig> =
        from_toml_path(rollup_config_path).context("Failed to read rollup configuration")?;

    let kernel_genesis = BasicKernelGenesisConfig {
        chain_state: serde_json::from_str(
            &std::fs::read_to_string(&kernel_genesis_paths.chain_state)
                .context("Failed to read chain state")?,
        )?,
    };

    let mock_rollup = CelestiaDemoRollup {};
    mock_rollup
        .create_new_rollup(
            rt_genesis_paths,
            kernel_genesis,
            rollup_config,
            prover_config,
        )
        .await
}

async fn new_rollup_with_mock_da(
    rt_genesis_paths: &GenesisPaths,
    kernel_genesis_paths: &BasicKernelGenesisPaths,
    rollup_config_path: &str,
    prover_config: RollupProverConfig,
) -> Result<Rollup<MockDemoRollup>, anyhow::Error> {
    debug!("Starting mock rollup with config {}", rollup_config_path);

    let rollup_config: RollupConfig<MockDaConfig> =
        from_toml_path(rollup_config_path).context("Failed to read rollup configuration")?;

    let kernel_genesis = BasicKernelGenesisConfig {
        chain_state: serde_json::from_str(
            &std::fs::read_to_string(&kernel_genesis_paths.chain_state)
                .context("Failed to read chain state")?,
        )?,
    };

    let mock_rollup = MockDemoRollup {};
    mock_rollup
        .create_new_rollup(
            rt_genesis_paths,
            kernel_genesis,
            rollup_config,
            prover_config,
        )
        .await
}
