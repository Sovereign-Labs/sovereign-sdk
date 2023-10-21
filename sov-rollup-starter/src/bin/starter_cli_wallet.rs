//! This binary defines a cli wallet for interacting
//! with the rollup.

use sov_rollup_interface::mocks::MockDaService;
use sov_rollup_interface::services::da::DaService;
#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    stf_starter::cli::run_wallet::<<MockDaService as DaService>::Spec>().await
}
