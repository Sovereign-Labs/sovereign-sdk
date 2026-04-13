use std::path::PathBuf;
use std::process::exit;

use anyhow::Context as _;
use clap::Parser;
use demo_stf::genesis_config::GenesisPaths;
use demo_stf::MultiAddressEvmSolana;
use sov_demo_rollup::zk::{self, InnerZkvm};
use sov_modules_api::capabilities::RollupHeight;
use sov_modules_api::execution_mode::Native;
use sov_modules_rollup_blueprint::logging::initialize_logging;
use sov_modules_rollup_blueprint::{FullNodeBlueprint, Rollup};
use sov_stf_runner::processes::{RollupProverConfig, RollupProverConfigDiscriminants};
use sov_stf_runner::{from_toml_path, RollupConfig};
use tracing::debug;

// Ensure exactly one DA feature is enabled
#[cfg(all(feature = "mock_da", feature = "celestia_da"))]
compile_error!("Both mock_da and celestia_da are enabled, but only one should be.");

#[cfg(all(feature = "mock_da", feature = "mock_da_external"))]
compile_error!("Both mock_da and mock_da_external are enabled, but only one should be.");

#[cfg(all(feature = "mock_da_external", feature = "celestia_da"))]
compile_error!("Both mock_da_external and celestia_da are enabled, but only one should be.");

#[cfg(all(
    not(feature = "mock_da"),
    not(feature = "celestia_da"),
    not(feature = "mock_da_external")
))]
compile_error!("No DA feature enabled. Enable exactly one of: mock_da, mock_da_external, celestia_da.");

// Ensure exactly one ZKVM feature is enabled
const _: () = {
    let risc0 = cfg!(feature = "risc0") as u8;
    let sp1 = cfg!(feature = "sp1") as u8;
    let mock_zkvm = cfg!(feature = "mock_zkvm") as u8;
    let count = risc0 + sp1 + mock_zkvm;

    assert!(
        count == 1,
        "Exactly one zkvm feature must be enabled: risc0, sp1, or mock_zkvm"
    );
};

/// Main demo runner. Initializes a DA chain, and starts a demo-rollup using the provided.
/// If you're trying to sign or submit transactions to the rollup, the `sov-cli` binary
/// is the one you want. You can run it `cargo run --bin sov-cli`.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// The path to the rollup config.
    #[cfg(feature = "mock_da")]
    #[arg(long, default_value = "configs/mock_rollup_config.toml")]
    rollup_config_path: String,

    /// The path to the rollup config.
    #[cfg(feature = "mock_da_external")]
    #[arg(long, default_value = "configs/mock_rollup_config.toml")]
    rollup_config_path: String,

    /// The path to the rollup config.
    #[cfg(feature = "celestia_da")]
    #[arg(long, default_value = "configs/celestia_rollup_config.toml")]
    rollup_config_path: String,

    /// The path to the genesis configs.
    #[cfg(any(feature = "mock_da", feature = "mock_da_external"))]
    #[arg(long, default_value = "../test-data/genesis/demo/mock")]
    genesis_config_dir: PathBuf,

    /// The path to the genesis configs.
    #[cfg(feature = "celestia_da")]
    #[arg(long, default_value = "../test-data/genesis/demo/celestia")]
    genesis_config_dir: PathBuf,

    /// Stops the rollup at a given height.
    #[arg(long, default_value = None)]
    stop_at_rollup_height: Option<u64>,

    /// Asserts that the rollup starts at a given height.
    #[arg(long, default_value = None)]
    start_at_rollup_height: Option<u64>,
}

#[tokio::main]
async fn main() {
    // Keep for preventing a opentelemtry export shutdown
    let _guard = initialize_logging();

    match run().await {
        Ok(_) => {
            tracing::debug!("Rollup execution complete. Shutting down.");
        }
        Err(e) => {
            tracing::error!(error = ?e, backtrace= e.backtrace().to_string(), "Rollup execution failed");
            exit(1);
        }
    }
}

async fn run() -> anyhow::Result<()> {
    let args = Args::parse();

    rustls::crypto::ring::default_provider()
        .install_default()
        .map_err(|e| anyhow::anyhow!("Failed to setup ring crypto provider: {e:?}"))?;

    let rollup_config_path = args.rollup_config_path.as_str();

    let prover_config_disc = parse_prover_config().expect("Failed to parse prover config");
    tracing::info!(
        ?prover_config_disc,
        "Running demo rollup with prover config"
    );

    let start_at_rollup_height = args.start_at_rollup_height.map(RollupHeight::new);
    let stop_at_rollup_height = args.stop_at_rollup_height.map(RollupHeight::new);

    #[cfg(feature = "mock_da")]
    {
        let prover_config = prover_config_disc
            .map(|config_disc| config_disc.into_config(zk::mock_da_host_args()));
        let rollup = new_rollup_with_mock_da(
            &GenesisPaths::from_dir(&args.genesis_config_dir),
            rollup_config_path,
            prover_config,
            start_at_rollup_height,
            stop_at_rollup_height,
        )
        .await
        .context("Failed to initialize MockDa rollup")?;
        rollup.run().await
    }

    #[cfg(feature = "mock_da_external")]
    {
        let prover_config = prover_config_disc
            .map(|config_disc| config_disc.into_config(zk::mock_da_host_args()));
        let rollup = new_rollup_with_external_mock_da(
            &GenesisPaths::from_dir(&args.genesis_config_dir),
            rollup_config_path,
            prover_config,
            start_at_rollup_height,
            stop_at_rollup_height,
        )
        .await
        .context("Failed to initialize ExternalMockDa rollup")?;
        rollup.run().await
    }

    #[cfg(feature = "celestia_da")]
    {
        let prover_config = prover_config_disc
            .map(|config_disc| config_disc.into_config(zk::celestia_host_args()));
        let rollup = new_rollup_with_celestia_da(
            &GenesisPaths::from_dir(&args.genesis_config_dir),
            rollup_config_path,
            prover_config,
            start_at_rollup_height,
            stop_at_rollup_height,
        )
        .await
        .context("Failed to initialize Celestia rollup")?;
        rollup.run().await
    }
}

fn parse_prover_config() -> anyhow::Result<Option<RollupProverConfigDiscriminants>> {
    if let Some(value) = option_env!("SOV_PROVER_MODE") {
        let config = std::str::FromStr::from_str(value).inspect_err(|&error| {
            tracing::error!(value, ?error, "Unknown `SOV_PROVER_MODE` value; aborting");
        })?;
        #[cfg(debug_assertions)]
        {
            if config == RollupProverConfigDiscriminants::Prove {
                tracing::warn!(prover_config = ?config, "Given RollupProverConfig might cause slow rollup progression if not compiled in release mode.");
            }
        }
        Ok(Some(config))
    } else {
        Ok(None)
    }
}

#[cfg(feature = "mock_da")]
async fn new_rollup_with_mock_da(
    rt_genesis_paths: &GenesisPaths,
    rollup_config_path: &str,
    prover_config: Option<RollupProverConfig<InnerZkvm>>,
    start_at_rollup_height: Option<RollupHeight>,
    stop_at_rollup_height: Option<RollupHeight>,
) -> anyhow::Result<Rollup<sov_demo_rollup::MockDemoRollup<Native>, Native>> {
    debug!(
        config_path = rollup_config_path,
        "Starting rollup on mock DA"
    );

    let rollup_config: RollupConfig<
        MultiAddressEvmSolana,
        sov_mock_da::storable::StorableMockDaService,
    > = from_toml_path(rollup_config_path).with_context(|| {
        format!("Failed to read rollup configuration from {rollup_config_path}")
    })?;

    let mock_rollup = sov_demo_rollup::MockDemoRollup::<Native>::default();
    mock_rollup
        .create_new_rollup(
            rt_genesis_paths,
            rollup_config,
            prover_config,
            start_at_rollup_height,
            stop_at_rollup_height,
            None,
        )
        .await
}

#[cfg(feature = "mock_da_external")]
async fn new_rollup_with_external_mock_da(
    rt_genesis_paths: &GenesisPaths,
    rollup_config_path: &str,
    prover_config: Option<RollupProverConfig<InnerZkvm>>,
    start_at_rollup_height: Option<RollupHeight>,
    stop_at_rollup_height: Option<RollupHeight>,
) -> anyhow::Result<Rollup<sov_demo_rollup::ExternalMockDemoRollup<Native>, Native>> {
    debug!(
        config_path = rollup_config_path,
        "Starting rollup on external-mock DA"
    );

    let rollup_config: RollupConfig<
        MultiAddressEvmSolana,
        sov_mock_da::storable::rpc::StorableMockDaClient,
    > = from_toml_path(rollup_config_path).with_context(|| {
        format!("Failed to read rollup configuration from {rollup_config_path}")
    })?;

    let mock_rollup = sov_demo_rollup::ExternalMockDemoRollup::<Native>::default();
    mock_rollup
        .create_new_rollup(
            rt_genesis_paths,
            rollup_config,
            prover_config,
            start_at_rollup_height,
            stop_at_rollup_height,
            None,
        )
        .await
}

#[cfg(feature = "celestia_da")]
async fn new_rollup_with_celestia_da(
    rt_genesis_paths: &GenesisPaths,
    rollup_config_path: &str,
    prover_config: Option<RollupProverConfig<InnerZkvm>>,
    start_at_rollup_height: Option<RollupHeight>,
    stop_at_rollup_height: Option<RollupHeight>,
) -> anyhow::Result<Rollup<sov_demo_rollup::CelestiaDemoRollup<Native>, Native>> {
    debug!(config_path = rollup_config_path, "Starting Celestia rollup");

    let rollup_config: RollupConfig<
        MultiAddressEvmSolana,
        sov_celestia_adapter::CelestiaService,
    > = from_toml_path(rollup_config_path).with_context(|| {
        format!("Failed to read rollup configuration from {rollup_config_path}")
    })?;

    let celestia_rollup = sov_demo_rollup::CelestiaDemoRollup::<Native>::default();
    celestia_rollup
        .create_new_rollup(
            rt_genesis_paths,
            rollup_config,
            prover_config,
            start_at_rollup_height,
            stop_at_rollup_height,
            None,
        )
        .await
}
