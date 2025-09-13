use crate::to_jsonrpsee_error_object;
use crate::Ethereum;
use alloy_rpc_types::pubsub::Params;
use alloy_rpc_types::pubsub::SubscriptionKind;
use alloy_rpc_types::Filter;
use jsonrpsee::types::{ErrorObjectOwned, Params as JRpcParams};
use jsonrpsee::PendingSubscriptionSink;
use jsonrpsee::SubscriptionMessage;
use jsonrpsee::{Extensions, SubscriptionSink};
use sov_address::{EthereumAddress, FromVmAddress};
pub use sov_evm::EthereumAuthenticator;
use sov_evm::Evm;
use sov_modules_api::capabilities::HasKernel;
use sov_modules_api::Spec;
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
    _log_filter: Box<Filter>,
    ethereum: Arc<Ethereum<S, Seq>>,
) where
    S: Spec,
    Seq: Sequencer<Spec = S>,
    S::Address: FromVmAddress<EthereumAddress>,
    Seq::Rt: HasKernel<S> + EthereumAuthenticator<S> + Default + Send + Sync + 'static,
{
    let evm = Evm::<S>::default();
    let state = &mut ethereum.sequencer.api_state().default_api_state_accessor();

    let pending_block = evm.pending_block(state);
    let mut prev_last_tx_index = pending_block.transactions.end;

    loop {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let state = &mut ethereum.sequencer.api_state().default_api_state_accessor();
        let pending_block = evm.pending_block(state);
        let curr_last_tx_index = pending_block.transactions.end;

        if curr_last_tx_index <= prev_last_tx_index {
            continue;
        }

        for index in prev_last_tx_index..curr_last_tx_index {
            let tx = evm.transaction(index, state).unwrap();
            // TODO
            let block = evm.get_maybe_sealed_block(tx.block_number, state);
            let receipt = evm.receipt(index, state).unwrap();
            // this is wrong
            let transaction_index = index - prev_last_tx_index;

            let logs = receipt
                .receipt
                .logs
                .iter()
                .enumerate()
                .map(|(tx_log_idx, log)| alloy_rpc_types::Log {
                    inner: log.clone(),
                    block_hash: block.hash(),
                    block_number: Some(block.number()),
                    block_timestamp: block.timestamp(),
                    transaction_hash: Some(*tx.signed_transaction().hash()),
                    transaction_index: Some(transaction_index),
                    // This is wrong
                    log_index: Some(receipt.log_index_start + tx_log_idx as u64),
                    removed: false,
                });

            for log in logs {
                let msg = SubscriptionMessage::new(
                    accepted_sink.method_name(),
                    accepted_sink.subscription_id(),
                    &log,
                )
                .unwrap();

                accepted_sink.send(msg).await.unwrap();
            }
        }
        prev_last_tx_index = curr_last_tx_index;
    }
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
