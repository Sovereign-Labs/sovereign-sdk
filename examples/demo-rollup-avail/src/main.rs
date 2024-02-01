use std::env;
use std::str::FromStr;

use sov_demo_rollup::new_rollup_with_avail_da;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{fmt, EnvFilter};

/// Main demo runner. Initialize a DA chain, and starts a demo-rollup using the config provided
/// (or a default config if not provided). Then start checking the blocks sent to the DA layer in
/// the main event loop.

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    // Initializing logging
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_str("info,sov_sequencer=warn").unwrap())
        .init();

    let rollup_config_path = env::args()
        .nth(1)
        .unwrap_or_else(|| "rollup_config.toml".to_string());
        
    let rollup = new_rollup_with_avail_da(&rollup_config_path).await?;
    rollup.run().await
}
