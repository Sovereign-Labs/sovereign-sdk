use anyhow::Context;
use clap::Parser;
use sov_address::MultiAddressEvm;
use sov_celestia_adapter::verifier::RollupParams;
use sov_celestia_adapter::CelestiaService;
use sov_demo_rollup::{ROLLUP_BATCH_NAMESPACE, ROLLUP_PROOF_NAMESPACE};
use sov_modules_rollup_blueprint::logging::initialize_logging;
use sov_stf_runner::{from_toml_path, RollupConfig};

/// Simple program description
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// The path to the rollup config.
    #[arg(long)]
    rollup_config_path: String,

    /// Number of rounds to check the DA service
    #[arg(long, default_value = "1")]
    rounds: usize,
}

/// Run tester of Celestia adapter.
/// To run it several times, pass `--rounds` parameter.
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _otel_guard = initialize_logging();
    let args = Args::parse();

    let rollup_config: RollupConfig<MultiAddressEvm, CelestiaService> =
        from_toml_path(&args.rollup_config_path).with_context(|| {
            format!(
                "Failed to read rollup configuration from {}",
                args.rollup_config_path
            )
        })?;

    tracing::info!("Rollup config: {:?}", rollup_config);

    let da_service = CelestiaService::new(
        rollup_config.da.clone(),
        RollupParams {
            rollup_batch_namespace: ROLLUP_BATCH_NAMESPACE,
            rollup_proof_namespace: ROLLUP_PROOF_NAMESPACE,
        },
    )
    .await;

    sov_celestia_adapter::checker::check_da_service(&da_service, args.rounds).await
}
