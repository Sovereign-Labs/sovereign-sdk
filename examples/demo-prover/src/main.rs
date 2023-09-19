use std::env;

use methods::ROLLUP_ELF;
use sov_demo_rollup::{new_rollup_with_celestia_da, DemoProverConfig};
use sov_risc0_adapter::host::Risc0Host;
use tracing::Level;

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    // If SKIP_PROVER is set, this means that we still compile and generate the riscV ELF
    // We execute the code inside the riscV but we don't prove it. This saves a significant amount of time
    // The primary benefit of doing this is to make sure we produce valid code that can run inside the
    // riscV virtual machine. Since proving is something we offload entirely to risc0, ensuring that
    // we produce valid riscV code and that it can execute is very useful.
    let prover_config = if env::var("SKIP_PROVER").is_ok() {
        DemoProverConfig::Execute
    } else {
        DemoProverConfig::Prove
    };
    // Initializing logging
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber)
        .map_err(|_err| eprintln!("Unable to set global default subscriber"))
        .expect("Cannot fail to set subscriber");

    // Same rollup_config.toml as used for the demo_rollup
    // When running from the demo-prover folder, the first argument can be pointed to ../demo-rollup/rollup_config.toml
    let rollup_config_path = env::args()
        .nth(1)
        .unwrap_or_else(|| "rollup_config.toml".to_string());
    println!("Read rollup config");

    let prover = Risc0Host::new(ROLLUP_ELF);

    let rollup =
        new_rollup_with_celestia_da(&rollup_config_path, Some((prover, prover_config))).await?;
    rollup.run().await?;

    Ok(())
}
