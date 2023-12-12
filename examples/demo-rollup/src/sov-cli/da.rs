use anyhow::Context;
use demo_stf::runtime::RuntimeSubcommand;
use sov_celestia_adapter::verifier::RollupParams;
use sov_celestia_adapter::CelestiaService;
use sov_demo_rollup::ROLLUP_PROOF_NAMESPACE;
use sov_demo_rollup::{CelestiaDemoRollup, ROLLUP_BATCH_NAMESPACE};
use sov_modules_api::cli::{FileNameArg, JsonStringArg};
use sov_stf_runner::from_toml_path;
use sov_stf_runner::RollupConfig;

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let rollup_config_path = "../celestia_rollup_config.toml";

    let rollup_config: RollupConfig<sov_celestia_adapter::CelestiaConfig> =
        from_toml_path(rollup_config_path).context("Failed to read rollup configuration")?;

    let service = CelestiaService::new(
        rollup_config.da.clone(),
        RollupParams {
            rollup_batch_namespace: ROLLUP_BATCH_NAMESPACE,
            rollup_proof_namespace: ROLLUP_PROOF_NAMESPACE,
        },
    )
    .await;
    Ok(())
}
