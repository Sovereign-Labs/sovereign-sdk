use std::str::FromStr;

use clap::Parser;
use sov_demo_rollup::{new_rollup_with_celestia_da, new_rollup_with_mock_da};
use sov_risc0_adapter::host::Risc0Host;
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
        .with(EnvFilter::from_str("info,sov_sequencer=warn").unwrap())
        .init();

    let args = Args::parse();
    let rollup_config_path = args.rollup_config_path.as_str();

    match args.da_layer.as_str() {
        "mock" => {
            let rollup = new_rollup_with_mock_da::<Risc0Host<'static>>(rollup_config_path, None)?;
            rollup.run().await
        }
        "celestia" => {
            let rollup =
                new_rollup_with_celestia_da::<Risc0Host<'static>>(rollup_config_path, None).await?;
            rollup.run().await
        }
        da => panic!("DA Layer not supported: {}", da),
    }
}
