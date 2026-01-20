mod eth_api_error;
mod filter_with_cursor;
mod revert_error;
mod rpc_invalid_transaction_error;
mod to_rpc_error;
mod utils;

pub use eth_api_error::{EthApiError, EthResult};
pub use filter_with_cursor::{FilterWithCursor, LogsWithMaybeCursor};
pub use revert_error::RevertError;
pub use rpc_invalid_transaction_error::RpcInvalidTransactionError;
pub use to_rpc_error::ToRpcError;
pub use utils::{internal_rpc_err, invalid_params_rpc_err, rpc_error_with_code};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq, Hash)]
pub struct LogWithExecutionTimestamp<L = alloy_rpc_types::Log> {
    #[serde(flatten)]
    pub log: L,
    #[serde(rename = "timeExecutedMs")]
    #[serde(with = "alloy_serde::quantity")]
    pub time_executed_ms: u64,
}
