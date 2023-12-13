use anyhow::Context;
use sov_celestia_adapter::verifier::RollupParams;
use sov_celestia_adapter::CelestiaService;
use sov_demo_rollup::ROLLUP_BATCH_NAMESPACE;
use sov_demo_rollup::ROLLUP_PROOF_NAMESPACE;
use sov_rollup_interface::services::da::DaService;
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

    println!("Start");
    let proof: Vec<u8> = vec![1, 2, 3];
    service.send_proof(proof).await?;
    //service.get_proofs_at(&proof).await?;

    println!("End");
    Ok(())
}
