use demo_stf::runtime::RuntimeSubcommand;
use sov_modules_api::cli::{FileNameArg, JsonStringArg};
use sov_modules_rollup_blueprint::WalletBlueprint;

// DA priority: mock_da > celestia_da (same fallback as the rest of the crate)
#[cfg(feature = "mock_da")]
use sov_demo_rollup::MockDemoRollup as DemoRollup;

#[cfg(all(feature = "celestia_da", not(feature = "mock_da")))]
use sov_demo_rollup::CelestiaDemoRollup as DemoRollup;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    DemoRollup::run_wallet::<
        RuntimeSubcommand<FileNameArg, _>,
        RuntimeSubcommand<JsonStringArg, _>,
    >()
    .await
}
