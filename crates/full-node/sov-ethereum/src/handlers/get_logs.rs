use crate::rpc_invalid_params;
use crate::rpc_limit_exceeded;
use crate::Ethereum;
use crate::EthereumAddress;
use crate::EthereumAuthenticator;
use crate::FromVmAddress;
use crate::HasKernel;
use crate::Sequencer;
use alloy_rpc_types::eth::Filter;
pub use cursor::Cursor;
use jsonrpsee::types::ErrorObjectOwned;
use jsonrpsee::types::Params as JRpcParams;
use jsonrpsee::Extensions;
use service::LogsService;
use sov_modules_api::Spec;
use sov_rpc_eth_types::LogWithExecutionTimestamp;
use sov_rpc_eth_types::{FilterWithCursor, LogsWithMaybeCursor};
use std::marker::PhantomData;
use std::sync::Arc;
mod cursor;
mod service;

pub struct LogHandlers<S: Spec, Seq: Sequencer<Spec = S>> {
    _phantom: PhantomData<(S, Seq)>,
}

impl<S, Seq> LogHandlers<S, Seq>
where
    S: Spec,
    Seq: Sequencer<Spec = S>,
    S::Address: FromVmAddress<EthereumAddress>,
    Seq::Rt: HasKernel<S> + EthereumAuthenticator<S> + Default + Send + Sync + 'static,
{
    pub async fn eth_get_logs(
        parameters: JRpcParams<'static>,
        ethereum: Arc<Ethereum<S, Seq>>,
        _: Extensions,
    ) -> Result<Vec<LogWithExecutionTimestamp>, ErrorObjectOwned> {
        // Force malformed filter payloads to return JSON-RPC -32602 (invalid params)
        // instead of bubbling up as internal deserialization errors.
        let filter = parameters
            .one::<Filter>()
            .map_err(|err| rpc_invalid_params(err.to_string()))?;

        let state = ethereum.api_state_accessor();
        let service = LogsService::<S, Seq>::new(
            filter,
            None,
            ethereum.extension.max_log_limit,
            state,
            ethereum.extension.response_size_limit,
        );
        let LogsWithMaybeCursor { logs, cursor } = service.logs_for_filter().await?;

        if cursor.is_some() {
            return Err(rpc_limit_exceeded(
                "Response size exceeds limit. Use eth_getLogsWithCursor or reduce the number of logs requested",
            ));
        }

        Ok(logs)
    }

    pub async fn eth_get_logs_with_cursor(
        parameters: JRpcParams<'static>,
        ethereum: Arc<Ethereum<S, Seq>>,
        _: Extensions,
    ) -> Result<LogsWithMaybeCursor, ErrorObjectOwned> {
        let state = ethereum.api_state_accessor();
        // Keep deserialization failures aligned with eth_getLogs: malformed payloads
        // should surface as JSON-RPC -32602 invalid params.
        let FilterWithCursor { cursor, filter } = parameters
            .one::<FilterWithCursor>()
            .map_err(|err| rpc_invalid_params(err.to_string()))?;
        let cursor = cursor.map(|s| Cursor::unpack(&s)).transpose()?;
        let service = LogsService::<S, Seq>::new(
            filter,
            cursor,
            ethereum.extension.max_log_limit,
            state,
            ethereum.extension.response_size_limit,
        );
        Ok(service.logs_for_filter().await?)
    }
}
