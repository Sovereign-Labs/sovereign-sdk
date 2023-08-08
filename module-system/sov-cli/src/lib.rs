#![deny(missing_docs)]
#![doc = include_str!("../README.md")]
use std::env;
use std::path::Path;

use directories::BaseDirs;
pub use sov_modules_api::clap;

/// Types and functionality storing and loading the persistent state of the wallet
pub mod wallet_state;
pub mod workflows;

const SOV_WALLET_DIR_ENV_VAR: &str = "SOV_WALLET_DIR";

/// The directory where the wallet is stored.
pub fn wallet_dir() -> Result<impl AsRef<Path>, anyhow::Error> {
    // First try to parse from the env variable
    if let Ok(val) = env::var(SOV_WALLET_DIR_ENV_VAR) {
        return Ok(val.into());
    }

    // Fall back to the user's home directory
    let dir = BaseDirs::new()
        .ok_or_else(|| anyhow::anyhow!("Could not find home directory. You can set a wallet directory using the {} environment variable", SOV_WALLET_DIR_ENV_VAR))?
        .home_dir()
        .join(".sov_cli_wallet");

    Ok(dir)
}
