use std::env;

use sov_demo_rollup::new_rollup_with_celestia_da;
use tracing::Level;

#[cfg(test)]
mod test_rpc;

/// Main demo runner. Initialize a DA chain, and starts a demo-rollup using the config provided
/// (or a default config if not provided). Then start checking the blocks sent to the DA layer in
/// the main event loop.

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    // Initializing logging
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .finish();

    tracing::subscriber::set_global_default(subscriber)
        .map_err(|_err| eprintln!("Unable to set global default subscriber"))
        .expect("Cannot fail to set subscriber");

    let rollup_config_path = env::args()
        .nth(1)
        .unwrap_or_else(|| "rollup_config.toml".to_string());

    let rollup = new_rollup_with_celestia_da(&rollup_config_path).await?;
    rollup.run().await
}
