//! This binary defines a cli wallet for interacting
//! with the rollup.

use sov_modules_api::cli::{FileNameArg, JsonStringArg};
use sov_modules_rollup_template::WalletTemplate;
use sov_rollup_starter::StarterRollup;
use stf_starter::runtime::RuntimeSubcommand;

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    StarterRollup::run_wallet::<
        RuntimeSubcommand<FileNameArg, _, _>,
        RuntimeSubcommand<JsonStringArg, _, _>,
    >()
    .await
}
