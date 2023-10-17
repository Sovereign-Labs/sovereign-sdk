mod api_v0;
mod db;
mod indexer;
mod metrics;
pub mod utils;

#[cfg(test)]
mod tests;

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use clap::Parser;
use db::Db;
use jsonrpsee::ws_client::WsClient;
use sov_celestia_adapter::verifier::address::CelestiaAddress;
use sov_modules_stf_template::{SequencerOutcome, TxEffect};
use sov_rollup_interface::rpc::{BatchResponse, SlotResponse, TxResponse};
use tracing::info;

use crate::indexer::index_blocks_loop;

type B = SequencerOutcome<CelestiaAddress>;
type Tx = TxEffect;

type AppState = Arc<AppStateInner>;

/// The application state, to which every request handler has access.
#[derive(Clone)]
pub struct AppStateInner {
    db: Db,
    rpc: Arc<WsClient>,
    base_url: String,
}

impl AppStateInner {
    /// Helps greatly with type inference when calling [`sov_rpc::RpcClient`]
    /// methods.
    fn rpc(
        &self,
    ) -> Arc<
        impl sov_ledger_rpc::client::RpcClient<
            SlotResponse<B, Tx>,
            BatchResponse<B, Tx>,
            TxResponse<Tx>,
        >,
    > {
        self.rpc.clone()
    }
}

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

    let app_state = Arc::new(AppStateInner {
        db,
        rpc,
        base_url: config.base_url.clone(),
    });

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

#[derive(Debug, Parser)]
pub struct Config {
    #[clap(long, default_value = "2")]
    polling_interval_in_secs: u64,
    #[clap(long, env)]
    db_connection_url: String,
    #[clap(long, default_value = "3010")]
    port: u16,
    #[clap(long, env, default_value = "ws://localhost:12345")]
    ledger_rpc_url: String,
    #[clap(long, default_value = "http://localhost:3010")]
    base_url: String,
}
