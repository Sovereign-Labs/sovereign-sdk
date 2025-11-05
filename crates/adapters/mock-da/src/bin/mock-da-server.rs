use clap::Parser;
use tracing_subscriber::EnvFilter;

use sov_mock_da::storable::rpc::start_server;
use sov_mock_da::storable::StorableMockDaService;
use sov_mock_da::{MockAddress, MockDaConfig};

// Start with cargo run --features="native"
#[derive(Parser, Debug)]
#[command(name = "mock-da-server")]
#[command(about = "Mock DA server for testing purposes", long_about = None)]
struct Cli {
    /// Host address to bind the server to
    #[arg(long, default_value = "127.0.0.1")]
    host: String,

    /// Port to bind the server to
    #[arg(short, long, default_value = "50051")]
    port: u16,

    /// Database connection string (e.g., "sqlite::memory:" or "sqlite:///path/to/db.sqlite?mode=rwc")
    #[arg(long, default_value = "sqlite::memory:")]
    db: String,

    /// Block time in milliseconds for periodic block production
    #[arg(long, default_value = "3000")]
    block_time_ms: u64,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("debug")),
        )
        .init();

    let cli = Cli::parse();

    let block_producing = sov_mock_da::BlockProducingConfig::Periodic {
        block_time_ms: cli.block_time_ms,
    };

    // Create DA configuration
    let config = MockDaConfig {
        connection_string: cli.db.clone(),
        sender_address: MockAddress::new([0u8; 32]),
        finalization_blocks: 0,
        block_producing,
        da_layer: None,
        randomization: None,
    };

    tracing::info!("Starting mock-da server with configuration:");
    tracing::info!("  Host: {}", cli.host);
    tracing::info!("  Port: {}", cli.port);
    tracing::info!("  Database: {}", cli.db);
    tracing::info!("  Block producing: {:?}", config.block_producing);

    let (shutdown_sender, shutdown_receiver) = tokio::sync::watch::channel(());
    let da_service = StorableMockDaService::from_config(config, shutdown_receiver).await;
    // Start the HTTP server
    let addr = start_server(da_service, &cli.host, cli.port).await?;

    tracing::info!("Mock DA server listening on {}", addr);
    tracing::info!("Server is running. Press Ctrl+C to stop.");

    // Wait for shutdown signal
    tokio::signal::ctrl_c().await?;
    tracing::info!("Shutting down mock-da server...");
    shutdown_sender.send(())?;
    Ok(())
}
