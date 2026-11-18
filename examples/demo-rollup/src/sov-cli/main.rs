use demo_stf::runtime::RuntimeSubcommand;
use sov_modules_api::cli::{FileNameArg, JsonStringArg};
use sov_modules_rollup_blueprint::WalletBlueprint;

#[cfg(not(any(
    feature = "mock_da",
    feature = "celestia_da",
    feature = "celestia_fibre_da"
)))]
compile_error!("enable at least one DA feature: `mock_da`, `celestia_da` or `celestia_fibre_da`");

// The wallet only signs/encodes transactions offline, so it is DA-agnostic; build it
// against whichever rollup this build provides, preferring mock_da, then celestia_da.
#[cfg(feature = "mock_da")]
type WalletRollup = sov_demo_rollup::MockDemoRollup<sov_modules_api::execution_mode::Native>;
#[cfg(all(feature = "celestia_da", not(feature = "mock_da")))]
type WalletRollup = sov_demo_rollup::CelestiaDemoRollup<sov_modules_api::execution_mode::Native>;
#[cfg(all(
    feature = "celestia_fibre_da",
    not(any(feature = "mock_da", feature = "celestia_da"))
))]
type WalletRollup =
    sov_demo_rollup::CelestiaFibreDemoRollup<sov_modules_api::execution_mode::Native>;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    WalletRollup::run_wallet::<
        RuntimeSubcommand<FileNameArg, _>,
        RuntimeSubcommand<JsonStringArg, _>,
    >()
    .await
}
