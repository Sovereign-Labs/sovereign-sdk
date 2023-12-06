use demo_stf::runtime::RuntimeSubcommand;
use sov_demo_rollup::CelestiaDemoRollup;
use sov_modules_api::cli::{JsonFileNameArg, JsonStringArg};
use sov_modules_rollup_blueprint::WalletBlueprint;

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    CelestiaDemoRollup::run_wallet::<
        RuntimeSubcommand<JsonFileNameArg, _, _>,
        RuntimeSubcommand<JsonStringArg, _, _>,
        _,
        _,
    >()
    .await
}
