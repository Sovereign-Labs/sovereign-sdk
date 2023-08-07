use std::env;
use std::path::{Path, PathBuf};

use directories::BaseDirs;
pub use sov_modules_api::clap;

pub mod wallet_state;
pub mod workflows;

const SOV_WALLET_DIR_ENV_VAR: &str = "SOV_WALLET_DIR";

/// The directory where the wallet is stored.
pub fn wallet_dir() -> Result<impl AsRef<Path>, anyhow::Error> {
    // First try to parse from the env variable
    if let Ok(val) = env::var(SOV_WALLET_DIR_ENV_VAR) {
        return Ok(
            PathBuf::try_from(val).map_err(|e: std::convert::Infallible| {
                anyhow::format_err!(
                    "Error parsing directory from the '{SOV_WALLET_DIR_ENV_VAR}' environment variable: {}",
                    e
                )
            })?,
        );
    }

    // Fall back to the user's home directory
    let dir = BaseDirs::new()
        .ok_or_else(|| anyhow::anyhow!("Could not find home directory. You can set a wallet directory using the {} environment variable", SOV_WALLET_DIR_ENV_VAR))?
        .home_dir()
        .join(".sov_cli_wallet");

    Ok(dir)
}
