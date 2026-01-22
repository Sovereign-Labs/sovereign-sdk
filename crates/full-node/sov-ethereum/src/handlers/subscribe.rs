use crate::handlers::subscribe::params::parse;
use crate::handlers::subscribe::params::validate;
use crate::handlers::subscribe::service::Streamer;
use crate::Ethereum;
use alloy_rpc_types::Filter;
use jsonrpsee::types::Params as JRpcParams;
use jsonrpsee::Extensions;
use jsonrpsee::PendingSubscriptionSink;
use sov_address::{EthereumAddress, FromVmAddress};
pub use sov_evm::EthereumAuthenticator;
use sov_modules_api::capabilities::HasKernel;
use sov_modules_api::Spec;
use sov_sequencer::Sequencer;
use std::sync::Arc;

mod params;
mod service;
pub(crate) mod watermark;

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) enum SubscriptionRequest {
    Logs(Box<Filter>),
    Heads,
}

pub async fn eth_subscribe<S, Seq>(
    parameters: JRpcParams<'static>,
    pending: PendingSubscriptionSink,
    ethereum: Arc<Ethereum<S, Seq>>,
    _: Extensions,
) -> jsonrpsee::core::SubscriptionResult
where
    S: Spec,
    Seq: Sequencer<Spec = S>,
    S::Address: FromVmAddress<EthereumAddress>,
    Seq::Rt: HasKernel<S> + EthereumAuthenticator<S> + Default + Send + Sync + 'static,
{
    let (kind, params) = match parse(parameters) {
        Ok(v) => v,
        Err(err) => return reject(pending, err).await,
    };

    let request = match validate(kind, params) {
        Ok(v) => v,
        Err(err) => return reject(pending, err).await,
    };

    let accepted = pending.accept().await?;
    let streamer = Streamer::new(accepted, ethereum.clone());

    tokio::spawn(async move {
        match request {
            SubscriptionRequest::Heads => streamer.new_heads().await,
            SubscriptionRequest::Logs(filter) => streamer.logs(filter).await,
        }
    });

    Ok(())
}

async fn reject(
    pending: PendingSubscriptionSink,
    err: impl Into<jsonrpsee::types::ErrorObjectOwned>,
) -> jsonrpsee::core::SubscriptionResult {
    pending.reject(err).await;
    Ok(())
}
