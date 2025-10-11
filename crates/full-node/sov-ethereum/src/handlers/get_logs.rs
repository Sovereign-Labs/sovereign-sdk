use super::service::LogsService;
use crate::Cursor;
use crate::Ethereum;
use crate::EthereumAddress;
use crate::EthereumAuthenticator;
use crate::FromVmAddress;
use crate::HasKernel;
use crate::Sequencer;
use alloy_rpc_types::eth::Filter;
use alloy_rpc_types::Log;
use jsonrpsee::types::ErrorObjectOwned;
use jsonrpsee::types::Params as JRpcParams;
use jsonrpsee::Extensions;
use sov_modules_api::Spec;
use sov_rpc_eth_types::{FilterWithCursor, LogsWithMaybeCursor};
use std::marker::PhantomData;
use std::sync::Arc;

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
    ) -> Result<Vec<Log>, ErrorObjectOwned> {
        let state = ethereum.api_state_accessor();
        let service = LogsService::<S, Seq>::new(
            parameters.one::<Filter>()?,
            None,
            ethereum.extension,
            state,
        );
        Ok(service.logs_for_filter().await?.logs)
    }

    pub async fn eth_get_logs_with_cursor(
        parameters: JRpcParams<'static>,
        ethereum: Arc<Ethereum<S, Seq>>,
        _: Extensions,
    ) -> Result<LogsWithMaybeCursor, ErrorObjectOwned> {
        let state = ethereum.api_state_accessor();
        let FilterWithCursor { cursor, filter } = parameters.one::<FilterWithCursor>()?;
        let cursor = cursor.map(|s| Cursor::unpack(&s)).transpose()?;
        let service = LogsService::<S, Seq>::new(filter, cursor, ethereum.extension, state);
        Ok(service.logs_for_filter().await?)
    }
}
