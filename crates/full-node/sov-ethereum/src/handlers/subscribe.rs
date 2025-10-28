use crate::handlers::subscribe::params::validate;
use crate::handlers::subscribe::service::Streamer;
use crate::Ethereum;
use alloy_rpc_types::pubsub::Params;
use alloy_rpc_types::pubsub::SubscriptionKind;
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
    let mut parameters = parameters.sequence();
    let kind: SubscriptionKind = parameters.next()?;
    let params: Params = parameters.optional_next()?.unwrap_or_default();

    let request = match validate(kind, params) {
        Ok(subscription_request) => subscription_request,
        Err(err) => {
            pending.reject(err).await;
            return Ok(());
        }
    };
    let accepted = pending.accept().await?;

    let streamer = Streamer::new(accepted, ethereum.clone());
    tokio::spawn(async move {
        match request {
            SubscriptionRequest::Heads => streamer.blocks().await,
            SubscriptionRequest::Logs(filter) => streamer.logs(filter).await,
        }
    });

    Ok(())
}
