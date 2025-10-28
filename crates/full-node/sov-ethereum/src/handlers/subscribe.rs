use crate::handlers::subscribe::service::ParamsValidationError;
use crate::handlers::subscribe::service::Streamer;
use crate::to_jsonrpsee_error_object;
use crate::Ethereum;
use alloy_rpc_types::pubsub::Params;
use alloy_rpc_types::pubsub::SubscriptionKind;
use alloy_rpc_types::FilterBlockOption;
use jsonrpsee::types::Params as JRpcParams;
use jsonrpsee::Extensions;
use jsonrpsee::PendingSubscriptionSink;
use sov_address::{EthereumAddress, FromVmAddress};
pub use sov_evm::EthereumAuthenticator;
use sov_modules_api::capabilities::HasKernel;
use sov_modules_api::Spec;
use sov_sequencer::Sequencer;
use std::sync::Arc;

mod service;
pub(crate) mod watermark;

use crate::handlers::ETH_RPC_ERROR;

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

    match kind {
        SubscriptionKind::NewHeads => {
            // NewHeads doesn't accept parameters
            if params != Params::None {
                let rpc_err = to_jsonrpsee_error_object(
                    ParamsValidationError::NewHeadsDoesNotAcceptParams,
                    ETH_RPC_ERROR,
                );
                pending.reject(rpc_err).await;
                return Ok(());
            }

            let accepted_sink = pending.accept().await?;
            tokio::spawn(async move {
                Streamer::new(accepted_sink, ethereum.clone())
                    .blocks()
                    .await
            });
        }
        SubscriptionKind::Logs => {
            let filter = match params {
                Params::Logs(filter) => {
                    if filter.block_option == FilterBlockOption::default() {
                        Ok(filter)
                    } else {
                        Err(ParamsValidationError::BlockOptionParam)
                    }
                }
                Params::Bool(_) => Err(ParamsValidationError::BoolParam),
                Params::None => Ok(Default::default()),
            };
            let log_filter = match filter {
                Ok(log_filter) => log_filter,
                Err(e) => {
                    let rpc_err = to_jsonrpsee_error_object(e, ETH_RPC_ERROR);
                    pending.reject(rpc_err).await;
                    return Ok(());
                }
            };

            let accepted_sink = pending.accept().await?;
            tokio::spawn(async move {
                Streamer::new(accepted_sink, ethereum.clone())
                    .logs(log_filter)
                    .await
            });
        }
        _ => {
            // NewPendingTransactions not supported
            let rpc_err = to_jsonrpsee_error_object(
                ParamsValidationError::OnlyLogAndNewHeadsSubscription,
                ETH_RPC_ERROR,
            );
            pending.reject(rpc_err).await;
            return Ok(());
        }
    }

    Ok(())
}
