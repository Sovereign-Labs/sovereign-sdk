#[path = "../lib.rs"]
pub mod backend;

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use backend::indexer::index_blocks_loop;
use backend::*;
use clap::Parser;
use sov_celestia_adapter::verifier::CelestiaSpec;
use sov_modules_api::DaSpec;
use sov_rollup_interface::mocks::MockDaSpec;
use tracing::info;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let config = Arc::new(Config::parse());

    // Initialize the database.
    let db = Db::new(&config.db_connection_url).await?;
    // Connect to the RPC provider.
    let rpc = Arc::new(
        jsonrpsee::ws_client::WsClientBuilder::new()
            .build(&config.ledger_rpc_url)
            .await?,
    );

    match config.da_layer {
        SupportedDaLayer::Celestia => {
            let app_state = Arc::new(AppStateInner::<CelestiaSpec>::new(
                db,
                rpc,
                config.base_url.clone(),
            ));
            inner_main(config, app_state).await
        }
        SupportedDaLayer::Mock => {
            let app_state = Arc::new(AppStateInner::<MockDaSpec>::new(
                db,
                rpc,
                config.base_url.clone(),
            ));
            inner_main(config, app_state).await
        }
    }
}

async fn inner_main<S>(config: Arc<Config>, app_state: AppState<S>) -> anyhow::Result<()>
where
    S: DaSpec + Send + Sync,
{
    let app = Router::new()
        .nest("/api/v0", api_v0::router(app_state.clone()))
        .nest("/metrics", metrics::router(metrics::Metrics {}));
    let socket_addr: SocketAddr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, config.port).into();

    let app_state_clone = app_state.clone();
    tokio::task::spawn(index_blocks_loop(
        app_state_clone,
        Duration::from_secs(config.polling_interval_in_secs),
    ));

    info!(socket_addr = socket_addr.to_string(), "Serving requests...");

    axum::Server::bind(&socket_addr)
        .serve(app.into_make_service())
        .await?;

    Ok(())
}
