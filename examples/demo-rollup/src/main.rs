use std::path::PathBuf;
use std::process::exit;

use anyhow::Context as _;
use clap::Parser;
use demo_stf::genesis_config::GenesisPaths;
use demo_stf::MultiAddressEvmSolana;
use sov_db::config::{
    RocksDbKind, RocksdbCfCustomization, RocksdbOptionsCustomization, RollupDbConfig,
    RollupDbConfigWithCustomizations, VersionedColumnFamilyKind,
};
use sov_demo_rollup::zk::{self, InnerZkvm};
use sov_modules_api::capabilities::RollupHeight;
use sov_modules_api::execution_mode::Native;
use sov_modules_rollup_blueprint::logging::initialize_logging;
use sov_modules_rollup_blueprint::{FullNodeBlueprint, Rollup};
use sov_stf_runner::processes::{RollupProverConfig, RollupProverConfigDiscriminants};
use sov_stf_runner::{from_toml_path, RollupConfig};
use tracing::debug;

// DA priority: mock_da > mock_da_external > celestia_da
// ZKVM priority: mock_zkvm > risc0 > sp1
// When multiple features are enabled (e.g. --all-features), the highest priority wins.

/// Main demo runner. Initializes a DA chain, and starts a demo-rollup using the provided.
/// If you're trying to sign or submit transactions to the rollup, the `sov-cli` binary
/// is the one you want. You can run it `cargo run --bin sov-cli`.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// The path to the rollup config.
    #[cfg(any(feature = "mock_da", feature = "mock_da_external"))]
    #[arg(long, default_value = "configs/mock_rollup_config.toml")]
    rollup_config_path: String,

    /// The path to the rollup config.
    #[cfg(all(
        feature = "celestia_da",
        not(feature = "mock_da"),
        not(feature = "mock_da_external")
    ))]
    #[arg(long, default_value = "configs/celestia_rollup_config.toml")]
    rollup_config_path: String,

    /// The path to the genesis configs.
    #[cfg(any(feature = "mock_da", feature = "mock_da_external"))]
    #[arg(long, default_value = "../test-data/genesis/demo/mock")]
    genesis_config_dir: PathBuf,

    /// The path to the genesis configs.
    #[cfg(all(
        feature = "celestia_da",
        not(feature = "mock_da"),
        not(feature = "mock_da_external")
    ))]
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
        let prover_config =
            prover_config_disc.map(|config_disc| config_disc.into_config(zk::mock_da_host_args()));
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

    #[cfg(all(feature = "mock_da_external", not(feature = "mock_da")))]
    {
        let prover_config =
            prover_config_disc.map(|config_disc| config_disc.into_config(zk::mock_da_host_args()));
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

    #[cfg(all(
        feature = "celestia_da",
        not(feature = "mock_da"),
        not(feature = "mock_da_external")
    ))]
    {
        let prover_config =
            prover_config_disc.map(|config_disc| config_disc.into_config(zk::celestia_host_args()));
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

/// Example: tune the live flat-state RocksDB instance for a workload with frequent small writes.
///
/// Call this after loading `RollupDbConfig` and before constructing
/// `NomtStorageManager::new_with_custom_config`.
/// This example does all three of...
/// - Setting db_wide options, setting cf-specific options, and setting table-specific options.
#[allow(dead_code)]
fn example_tune_live_nomt_table_for_small_writes(
    storage: RollupDbConfig,
) -> RollupDbConfigWithCustomizations {
    RollupDbConfigWithCustomizations::new(storage)
        .with_rocksdb_options(RocksdbOptionsCustomization::new(|db_kind, db_opts| {
            if db_kind != RocksDbKind::FlatStateLive {
                return;
            }

            // Keep background flushing steady so bursts of small writes do not stall as hard.
            db_opts.set_bytes_per_sync(1 << 20);
            db_opts.set_max_background_jobs(4);
        }))
        // Set some options at the cf and table levels. Note that this function is shared
        // across all DBs, so you'll want to filter on db_kind if you need separate
        // cf-level options for different DBs.
        .with_rocksdb_cf_options(RocksdbCfCustomization::new(
            |db_kind, _cf_name, versioned_kind, builder| {
                // Apply these changes to the live nomt tables. We could also filter on
                // cf_name if we wanted.
                // Note that all 3 of db_kind, cf_name, and versioned_kind are provided to
                // support easy filtering but can be safely ignored if you want to apply
                // the changes indiscriminately.
                if db_kind != RocksDbKind::FlatStateLive
                    || versioned_kind != Some(VersionedColumnFamilyKind::Live)
                {
                    return;
                }

                // Set some options at the cf level.
                let cf_opts = builder.options_mut();
                cf_opts.set_write_buffer_size(8 * 1024 * 1024);
                cf_opts.set_target_file_size_base(64 * 1024 * 1024);

                // Set some options at the table level.
                let table_opts = builder.block_based_table_options_mut();
                table_opts.set_block_size(4 * 1024);
                table_opts.set_cache_index_and_filter_blocks(true);
                table_opts.set_pin_l0_filter_and_index_blocks_in_cache(true);
                table_opts.set_whole_key_filtering(true);
            },
        ))
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

#[cfg(all(feature = "mock_da_external", not(feature = "mock_da")))]
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

#[cfg(all(
    feature = "celestia_da",
    not(feature = "mock_da"),
    not(feature = "mock_da_external")
))]
async fn new_rollup_with_celestia_da(
    rt_genesis_paths: &GenesisPaths,
    rollup_config_path: &str,
    prover_config: Option<RollupProverConfig<InnerZkvm>>,
    start_at_rollup_height: Option<RollupHeight>,
    stop_at_rollup_height: Option<RollupHeight>,
) -> anyhow::Result<Rollup<sov_demo_rollup::CelestiaDemoRollup<Native>, Native>> {
    debug!(config_path = rollup_config_path, "Starting Celestia rollup");

    let rollup_config: RollupConfig<MultiAddressEvmSolana, sov_celestia_adapter::CelestiaService> =
        from_toml_path(rollup_config_path).with_context(|| {
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
