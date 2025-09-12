use crate::to_jsonrpsee_error_object;
use crate::Ethereum;
use alloy_primitives::Address;
use alloy_primitives::{Bytes, B256};
use alloy_rpc_types::pubsub::Params;
use alloy_rpc_types::pubsub::SubscriptionKind;
use alloy_rpc_types::{Filter, Log};
use jsonrpsee::core::SubscriptionError;
use jsonrpsee::types::{ErrorCode, ErrorObjectOwned, Params as JRpcParams};
use jsonrpsee::PendingSubscriptionSink;
use jsonrpsee::SubscriptionMessage;
use jsonrpsee::{Extensions, SubscriptionSink};
use reth_primitives::LogData;
use sov_address::{EthereumAddress, FromVmAddress};
pub use sov_evm::EthereumAuthenticator;
use sov_evm::Evm;
use sov_evm::RlpEvmTransaction;
use sov_modules_api::capabilities::HasKernel;
use sov_modules_api::{RawTx, Spec};
use sov_sequencer::Sequencer;
use std::sync::Arc;
use std::time::Duration;

use crate::handlers::ETH_RPC_ERROR;

#[derive(Debug, Clone, serde::Deserialize)]
struct EthSubscribe {
    kind: SubscriptionKind,
    params: Params,
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
    let log_filter = match validate_params_for_log_subscription(parameters) {
        Ok(log_filter) => log_filter,
        Err(e) => {
            pending.reject(ErrorObjectOwned::from(e)).await;
            return Ok(());
        }
    };

    let accepted_sink = pending.accept().await?;

    let _task = tokio::spawn(async move {
        stream_logs(accepted_sink, log_filter, ethereum.clone()).await;
    });

    Ok(())
}

async fn stream_logs<S, Seq>(
    accepted_sink: SubscriptionSink,
    log_filter: Box<Filter>,
    _ethereum: Arc<Ethereum<S, Seq>>,
) where
    S: Spec,
    Seq: Sequencer<Spec = S>,
    S::Address: FromVmAddress<EthereumAddress>,
    Seq::Rt: HasKernel<S> + EthereumAuthenticator<S> + Default + Send + Sync + 'static,
{
    tokio::time::sleep(Duration::from_millis(1)).await;

    let log = Log {
        inner: alloy_primitives::Log {
            address: Address::parse_checksummed("0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045", None)
                .unwrap(),

            data: LogData::new_unchecked(vec![], Default::default()),
        },
        block_hash: None,
        block_number: None,
        block_timestamp: None,
        transaction_hash: None,
        transaction_index: None,
        log_index: None,
        removed: false,
    };

    let msg = SubscriptionMessage::new(
        accepted_sink.method_name(),
        accepted_sink.subscription_id(),
        &log,
    )
    .unwrap();

    accepted_sink.send(msg).await.unwrap();
}

fn validate_params_for_log_subscription(
    parameters: JRpcParams<'static>,
) -> Result<Box<Filter>, ErrorObjectOwned> {
    let eth_subscribe = parameters.parse::<EthSubscribe>()?;

    let log_filter = match &eth_subscribe.kind {
        SubscriptionKind::Logs => match eth_subscribe.params {
            Params::Logs(filter) => filter,
            Params::Bool(_) => {
                return Err(to_jsonrpsee_error_object(
                    "Boolean parameters are not supported in LOG subscriptions.",
                    ETH_RPC_ERROR,
                ));
            }
            _ => Default::default(),
        },
        _ => {
            return Err(to_jsonrpsee_error_object(
                "Only LOG subscriptions are supported.",
                ETH_RPC_ERROR,
            ))
        }
    };

    Ok(log_filter)
}
