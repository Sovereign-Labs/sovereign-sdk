use anyhow::Context;
use rollup_template::rollup::Rollup;
use sov_celestia_adapter::{types::NamespaceId, verifier::RollupParams};
use sov_stf_runner::{from_toml_path, RollupConfig};
// use rollup_template::
use tracing_subscriber::{filter::LevelFilter, EnvFilter};

type DaConfig = sov_celestia_adapter::DaServiceConfig;
type DaService = sov_celestia_adapter::CelestiaService;
const ROLLUP_NAMESPACE: NamespaceId = NamespaceId([11; 8]);

// type

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
    let rollup_config: RollupConfig<DaConfig> =
        from_toml_path("rollup_config.toml").context("Failed to read rollup configuration")?;
    let da_service = DaService::new(
        rollup_config.da,
        RollupParams {
            namespace: ROLLUP_NAMESPACE,
        },
    )
    .await;
    let genesis_config =
        serde_json::from_str(std::fs::read_to_string("genesis_config.json")?.as_str())?;

    let rollup = Rollup::new(da_service, genesis_config, rollup_config, None)?;

    Ok(())
}
