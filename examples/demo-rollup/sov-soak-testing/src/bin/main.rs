use clap::Parser;
use sov_modules_api::prelude::tracing;
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
        let setup = sov_soak_testing::setup_roles_and_config_sp1();
        let rollup = sov_soak_testing::setup_rollup_with_network_proving(
            args.storage_path.into(),
            args.axum_port,
            setup,
            args.db_connection_url,
        )
        .await;
        wait_for_shutdown_and_stop(rollup.shutdown_sender.subscribe(), || async {
            rollup.shutdown().await
        })
        .await?;
    } else {
        let setup = sov_soak_testing::setup_roles_and_config();
        let rollup = sov_soak_testing::setup_rollup(
            args.storage_path.into(),
            args.axum_port,
            setup,
            args.db_connection_url,
        )
        .await;
        wait_for_shutdown_and_stop(rollup.shutdown_sender.subscribe(), || async {
            rollup.shutdown().await
        })
        .await?;
    };

    Ok(())
}

async fn wait_for_shutdown_and_stop<F, Fut, T>(
    mut shutdown_recv: tokio::sync::watch::Receiver<()>,
    shutdown_fn: F,
) -> Result<(), anyhow::Error>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<T, anyhow::Error>>,
{
    let mut terminate = tokio::signal::unix::signal(SignalKind::terminate())
        .expect("Failed to set up SIGTERM handler");
    let mut quit =
        tokio::signal::unix::signal(SignalKind::quit()).expect("Failed to set up SIGQUIT handler");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => tracing::info!("Received Ctrl+C"),
        _ = terminate.recv() => tracing::info!("Received SIGTERM"),
        _ = quit.recv() => tracing::info!("Received SIGQUIT"),
        // might not be desired because soak tests are intended to run continously until we stop
        // them, if we got a shutdown msg something probably went wrong :-)
        _ = shutdown_recv.changed() => tracing::warn!("Rollup execution finished, this might not be desired!!"),
    }

    shutdown_fn().await?;
    Ok(())
}
