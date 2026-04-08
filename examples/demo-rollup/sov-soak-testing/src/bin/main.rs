use std::sync::Arc;

use clap::Parser;
use sov_modules_api::prelude::tracing;
use sov_modules_rollup_blueprint::FullNodeBlueprint;
use sov_soak_testing::RollupProverConfig;
use sov_test_utils::test_rollup::RollupBuilder;
use tokio::signal::unix::SignalKind;

#[derive(Parser)]
struct Args {
    #[arg(short, long, default_value_t = 12346)]
    /// The port that the axum API server will listen on.
    /// Defaults to 12346.
    axum_port: u16,

    #[arg(short, long, default_value = "soak_data/")]
    storage_path: String,

    /// DB connection URL for the sequencer.
    /// Allows the sequencer to connect to a remote postgres database.
    /// If not provided the sequencer will use rocksdb.
    #[arg(short, long)]
    db_connection_url: Option<String>,

    /// Enable SP1 network proving.
    /// Requires NETWORK_PRIVATE_KEY env var to be set.
    /// The rollup will submit proofs to the Succinct proving network.
    #[arg(long)]
    network_proving: bool,
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let _guard = sov_modules_rollup_blueprint::logging::initialize_logging();
    let args = Args::parse();
    std::fs::create_dir_all(&args.storage_path)?;

    if args.network_proving {
        let builder = sov_soak_testing::create_sp1_rollup_builder(
            args.storage_path.into(),
            args.axum_port,
            args.db_connection_url,
        )
        .set_config(|config| {
            // Enable witness generation so proofs can be submitted to the network.
            // The actual host args are unused by NetworkProverService.
            config.rollup_prover_config = Some(RollupProverConfig::Prove(Arc::new(
                *sp1::SP1_GUEST_MOCK_ELF,
            )));
        });
        start_and_wait(builder).await?;
    } else {
        let setup = sov_soak_testing::setup_roles_and_config();
        let builder = sov_soak_testing::create_mock_rollup_builder(
            args.storage_path.into(),
            args.axum_port,
            &setup,
            args.db_connection_url,
        )
        .set_config(|config| {
            config.rollup_prover_config = None;
        });
        start_and_wait(builder).await?;
    };

    Ok(())
}

async fn start_and_wait<R>(builder: RollupBuilder<R>) -> Result<(), anyhow::Error>
where
    R: FullNodeBlueprint<
            sov_modules_api::execution_mode::Native,
            DaService = sov_mock_da::storable::StorableMockDaService,
        > + Default
        + 'static,
    R::Spec: sov_modules_api::Spec<Da = sov_mock_da::MockDaSpec>,
{
    let rollup = builder.start().await.expect("Impossible to start rollup");
    let mut shutdown_recv = rollup.shutdown_sender.subscribe();

    let mut terminate = tokio::signal::unix::signal(SignalKind::terminate())
        .expect("Failed to set up SIGTERM handler");
    let mut quit =
        tokio::signal::unix::signal(SignalKind::quit()).expect("Failed to set up SIGQUIT handler");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => tracing::info!("Received Ctrl+C"),
        _ = terminate.recv() => tracing::info!("Received SIGTERM"),
        _ = quit.recv() => tracing::info!("Received SIGQUIT"),
        _ = shutdown_recv.changed() => tracing::warn!("Rollup execution finished, this might not be desired!!"),
    }

    rollup.shutdown().await?;
    Ok(())
}
