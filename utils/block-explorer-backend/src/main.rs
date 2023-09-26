mod db;
pub(crate) mod models;
mod routing;

use std::{
    net::{Ipv4Addr, SocketAddr, SocketAddrV4},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

use axum::Router;
use clap::Parser;
use db::Db;
use sov_celestia_adapter::verifier::address::CelestiaAddress;
use sov_db::ledger_db::LedgerDB;
use sov_modules_stf_template::{SequencerOutcome, TxEffect};
use sov_rollup_interface::rpc::{LedgerRpcProvider, QueryMode, TxIdentifier};
use tracing::{info, trace, warn};

type AppState = Arc<AppStateInner>;

#[derive(Clone)]
pub struct AppStateInner {
    db: Db,
    rpc: LedgerDB,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let config = Config::parse();

    // Initialize the database.
    let db = Db::new(&config.db_connection_url).await?;
    // Connect to the RPC provider, which ironically in this case, is actually
    // the ledger RocksDB (so it's not really an RPC).
    // TODO: connect to the node via RPC.
    let rpc = LedgerDB::with_path(&config.ledger_db_path).expect("Failed to open ledger db");
    let app_state = Arc::new(AppStateInner { db, rpc });

    let app = Router::new().nest("/api/v0", routing::api_v0_router(app_state.clone()));
    let socket_addr: SocketAddr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, config.port).into();

    let app_state_clone = app_state.clone();
    tokio::task::spawn(index_blocks(
        app_state_clone,
        Duration::from_secs(config.polling_interval_in_secs),
    ));

    info!(socket_addr = socket_addr.to_string(), "Serving requests...");

    axum::Server::bind(&socket_addr)
        .serve(app.into_make_service())
        .await?;

    Ok(())
}

async fn index_blocks(app_state: AppState, polling_interval: Duration) {
    type B = SequencerOutcome<CelestiaAddress>;
    type Tx = TxEffect;

    loop {
        trace!(
            polling_interval_in_msecs = polling_interval.as_millis(),
            "Going to sleep before new polling cycle"
        );
        // Sleep for a bit. We wouldn't want to spam the node.
        tokio::time::sleep(polling_interval).await;

        // TODO: retry and error handling.
        let chain_head = if let Some(head) = app_state.rpc.get_head::<B, Tx>().unwrap() {
            head
        } else {
            warn!("`get_head` returned no data, can't index blocks.");
            continue;
        };

        let chain_block_range = 0..=chain_head.number;
        for i in chain_block_range {
            let block = app_state
                .rpc
                .get_slot_by_number::<B, Tx>(i, QueryMode::Standard)
                .unwrap();
            if let Some(block) = block {
                let block_json = serde_json::to_value(block).unwrap();
                app_state.db.upsert_blocks(&[&block_json]).await.unwrap();
            } else {
                warn!("`get_slot_by_number` returned no data for block {}", i);
            }
        }

        // Finally, insert the chain head.
        let chain_head_json = serde_json::to_value(chain_head).unwrap();
        app_state
            .db
            .upsert_blocks(&[&chain_head_json])
            .await
            .unwrap();

        let txs = app_state
            .rpc
            .get_transactions::<Tx>(
                &[TxIdentifier::Number(1)],
                sov_rollup_interface::rpc::QueryMode::Full,
            )
            .unwrap();
        println!("Transactions: {:?}", txs);
    }
}

#[derive(Debug, Parser)]
struct Config {
    #[clap(long, default_value = "2")]
    polling_interval_in_secs: u64,
    #[clap(long, env)]
    db_connection_url: String,
    #[clap(long, default_value = "3010")]
    port: u16,
    #[clap(long)]
    ledger_db_path: PathBuf,
}
