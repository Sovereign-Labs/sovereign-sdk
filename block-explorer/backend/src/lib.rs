pub mod api_v0;
mod db;
pub mod indexer;
pub mod metrics;
pub mod utils;

use std::{marker::PhantomData, sync::Arc};

use clap::{Parser, ValueEnum};
pub use db::Db;
use jsonrpsee::ws_client::WsClient;
use sov_modules_api::DaSpec;
use sov_modules_stf_template::{SequencerOutcome, TxEffect};
use sov_rollup_interface::rpc::{BatchResponse, SlotResponse, TxResponse};

pub type AppState<S> = Arc<AppStateInner<S>>;

pub type BatchReceipt<S> = SequencerOutcome<<S as DaSpec>::Address>;
pub type TxReceipt = TxEffect;

/// The application state, to which every request handler has access.
#[derive(Clone)]
pub struct AppStateInner<S> {
    phantom: PhantomData<S>,
    pub db: Db,
    pub rpc: Arc<WsClient>,
    pub base_url: String,
}

impl<S> AppStateInner<S>
where
    S: DaSpec,
{
    pub fn new(db: Db, rpc: Arc<WsClient>, base_url: String) -> Self {
        Self {
            phantom: PhantomData,
            db,
            rpc,
            base_url,
        }
    }

    /// Helps greatly with type inference when calling
    /// [`sov_ledger_rpc::client::RpcClient`] methods.
    pub fn rpc(
        &self,
    ) -> Arc<
        impl sov_ledger_rpc::client::RpcClient<
            SlotResponse<BatchReceipt<S>, TxReceipt>,
            BatchResponse<BatchReceipt<S>, TxReceipt>,
            TxResponse<TxReceipt>,
        >,
    > {
        self.rpc.clone()
    }
}

#[derive(Debug, Parser)]
pub struct Config {
    /// How long to wait between polling chain head updates.
    #[clap(long, default_value = "2")]
    pub polling_interval_in_secs: u64,
    /// The database connection URL.
    #[clap(long, env)]
    pub db_connection_url: String,
    /// The port to listen on.
    #[clap(long, default_value = "3010")]
    pub port: u16,
    /// The URL of the ledger JSON-RPC API.
    #[clap(long, env, default_value = "ws://localhost:12345")]
    pub ledger_rpc_url: String,
    /// The base URL of the API, used for link and URL generation.
    #[clap(long, default_value = "http://localhost:3010")]
    pub base_url: String,
    /// What DA layer the ledger is running on.
    #[clap(long, default_value = "celestia")]
    pub da_layer: SupportedDaLayer,
}

#[derive(Debug, Copy, Clone, ValueEnum)]
#[clap(rename_all = "kebab-case")]
pub enum SupportedDaLayer {
    Celestia,
    Mock,
}
