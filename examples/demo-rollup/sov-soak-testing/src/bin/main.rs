use clap::Parser;
use sov_modules_api::prelude::tracing;
use sov_soak_testing::setup_rollup;
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
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let _guard = sov_modules_rollup_blueprint::logging::initialize_logging();
    let args = Args::parse();
    std::fs::create_dir_all(&args.storage_path)?;
    let setup = sov_soak_testing::setup_roles_and_config();
    let rollup = setup_rollup(
        args.storage_path.into(),
        args.axum_port,
        setup,
        args.db_connection_url,
    )
    .await;
    // There's a race condition here - when tasks fail at startup
    // we might not have been subscribed to the sender yet so the rollup haults
    // and we remain waiting on the `select!` below until we ctrl+c
    let mut shutdown_recv = rollup.shutdown_sender.subscribe();

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

    rollup.shutdown().await?;

    Ok(())
}
