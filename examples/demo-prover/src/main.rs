use std::env;

use methods::ROLLUP_ELF;
use sov_demo_rollup::{new_rollup_with_celestia_da, DemoProverConfig};
use sov_risc0_adapter::host::Risc0Host;
use tracing::info;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    // If SKIP_PROVER is set, We still compile and run the zkVM code inside of an emulator without generating
    // a proof. This dramatically reduces the runtime of the prover, while still ensuring that our rollup
    // code is valid and operates as expected.
    let prover_config = if env::var("SKIP_PROVER").is_ok() {
        DemoProverConfig::Execute
    } else {
        DemoProverConfig::Prove
    };

    // Initializing logging
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::builder()
                .with_default_directive(LevelFilter::INFO.into()) // If no logging config is set. default to `info` level logs
                .from_env_lossy(), // Parse the log level from the RUST_LOG env var if set
        ) // Try to override logging config from RUST_LOG env var
        .finish();
    tracing::subscriber::set_global_default(subscriber)
        .map_err(|_err| eprintln!("Unable to set global default subscriber"))
        .expect("Cannot fail to set subscriber");

    // The format of the demo-prover config file is identircal to that of demo-rollup.
    // When running from the demo-prover folder, the first argument can be pointed to ../demo-rollup/rollup_config.toml
    let rollup_config_path = env::args()
        .nth(1)
        .unwrap_or_else(|| "rollup_config.toml".to_string());
    info!("Reading rollup config from {rollup_config_path}");

    // Initialize the rollup. For this demo, we use Risc0 and Celestia.
    let prover = Risc0Host::new(ROLLUP_ELF);
    let rollup =
        new_rollup_with_celestia_da(&rollup_config_path, Some((prover, prover_config))).await?;
    rollup.run().await?;

    Ok(())
}
