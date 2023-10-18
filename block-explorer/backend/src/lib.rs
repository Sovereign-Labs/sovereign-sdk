pub mod api_v0;
mod db;
pub mod indexer;
pub mod metrics;
pub mod utils;

use std::sync::Arc;

use clap::Parser;
pub use db::Db;
use jsonrpsee::ws_client::WsClient;
use sov_celestia_adapter::verifier::address::CelestiaAddress;
use sov_modules_stf_template::{SequencerOutcome, TxEffect};
use sov_rollup_interface::rpc::{BatchResponse, SlotResponse, TxResponse};

type B = SequencerOutcome<CelestiaAddress>;
type Tx = TxEffect;

pub type AppState = Arc<AppStateInner>;

/// The application state, to which every request handler has access.
#[derive(Clone)]
pub struct AppStateInner {
    pub db: Db,
    pub rpc: Arc<WsClient>,
    pub base_url: String,
}

impl AppStateInner {
    /// Helps greatly with type inference when calling
    /// [`sov_ledger_rpc::client::RpcClient`] methods.
    pub fn rpc(
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
