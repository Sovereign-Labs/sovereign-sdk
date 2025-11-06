use crate::to_jsonrpsee_error_object;
use crate::Ethereum;
use alloy_consensus::Sealed;
use alloy_rpc_types::Header;
use alloy_rpc_types::pubsub::Params;
use alloy_rpc_types::pubsub::SubscriptionKind;
use alloy_rpc_types::Filter;
use alloy_rpc_types::FilterBlockOption;
use jsonrpsee::types::Params as JRpcParams;
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
use thiserror::Error;

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

     match validate_params_subscription(kind, params) {
        Ok(SupportedSubscriptionParams::Logs(filter)) =>  {
            let accepted_sink = pending.accept().await?;
            let _task = tokio::spawn(async move {
                stream_logs(accepted_sink, filter, ethereum.clone()).await;
            });
        }
        Ok(SupportedSubscriptionParams::NewHeads) =>  {
            let accepted_sink = pending.accept().await?;
            let _task = tokio::spawn(async move {
                stream_new_heads(accepted_sink, ethereum.clone()).await;
            });
        }
        Err(e) => {
            let rpc_err = to_jsonrpsee_error_object(e, ETH_RPC_ERROR);
            pending.reject(rpc_err).await;
            return Ok(());
        }
    };

    
    Ok(())
}

async fn stream_logs<S, Seq>(
    accepted_sink: SubscriptionSink,
    filter: Box<Filter>,
    ethereum: Arc<Ethereum<S, Seq>>,
) where
    S: Spec,
    Seq: Sequencer<Spec = S>,
    S::Address: FromVmAddress<EthereumAddress>,
    Seq::Rt: HasKernel<S> + EthereumAuthenticator<S> + Default + Send + Sync + 'static,
{
    let evm = Evm::<S>::default();
    let state = &mut ethereum.api_state_accessor();

    let pending_block = evm.pending_block(state);
    let mut prev_last_tx_index = pending_block.transactions.end;

    // Fetch the initial block. If it’s stale, it will be replaced below.
    let start_block = pending_block.header.number - 1;
    let Some(mut block) = evm.get_maybe_sealed_block(start_block, state) else {
        tracing::error!(start_block, "Block does not exist");
        return;
    };

    let state_updates = &mut ethereum.sequencer.api_state().checkpoint_receiver();

    while state_updates.changed().await.is_ok() {
        let state = &mut ethereum.api_state_accessor();

        let pending_block = evm.pending_block(state);
        let curr_last_tx_index = pending_block.transactions.end;

        if curr_last_tx_index <= prev_last_tx_index {
            continue;
        }

        for index in prev_last_tx_index..curr_last_tx_index {
            let Some(receipt) = evm.receipt(index, state) else {
                // This can happen if the state was pruned.
                tracing::error!(index, "Receipt does not exist");
                return;
            };

            if block.number() != receipt.block_number {
                match evm.get_maybe_sealed_block(receipt.block_number, state) {
                    Some(b) => block = b,
                    None => {
                        tracing::error!(
                            block_number = receipt.block_number,
                            "Block does not exist"
                        );
                        return;
                    }
                }
            }

            let transaction_index = index - block.transactions_start();

            for (log_index_in_tx, log) in receipt.receipt.logs.into_iter().enumerate() {
                if filter.matches(&log) {
                    let rpc_log = alloy_rpc_types::Log {
                        inner: log,
                        block_hash: block.hash(),
                        block_number: Some(block.number()),
                        block_timestamp: Some(block.timestamp()),
                        transaction_hash: Some(receipt.transaction_hash),
                        transaction_index: Some(receipt.transaction_index),
                        log_index: Some(receipt.log_index_start + log_index_in_tx as u64),
                        removed: false,
                    };

                    assert_eq!(receipt.transaction_index, transaction_index);

                    let msg = SubscriptionMessage::new(
                        accepted_sink.method_name(),
                        accepted_sink.subscription_id(),
                        &rpc_log,
                    )
                    .unwrap_or_else(|err| {
                        panic!("Impossible: can't serialize log. Log: {rpc_log:?}, Err: {err:?}",)
                    });

                    if let Err(err) = accepted_sink.send(msg).await {
                        tracing::info!(%err, "The subscription client disconnected from the server.");
                        return;
                    }
                }
            }
        }
        prev_last_tx_index = curr_last_tx_index;
    }
}


async fn stream_new_heads<S, Seq>(
    accepted_sink: SubscriptionSink,
    ethereum: Arc<Ethereum<S, Seq>>,
) where
    S: Spec,
    Seq: Sequencer<Spec = S>,
    S::Address: FromVmAddress<EthereumAddress>,
    Seq::Rt: HasKernel<S> + EthereumAuthenticator<S> + Default + Send + Sync + 'static,
{
    let evm = Evm::<S>::default();
    let state = &mut ethereum.api_state_accessor();

    
    let mut last_sent_block_number = *evm.block_numbers(state).end();

    let state_updates = &mut ethereum.sequencer.api_state().checkpoint_receiver();

    while state_updates.changed().await.is_ok() {
        let state = &mut ethereum.api_state_accessor();
        let all_block_numbers = evm.block_numbers(state);
        let block_number = *all_block_numbers.end();
        if block_number < last_sent_block_number {
            tracing::error!(block_number, last_sent_block_number, "Block number is less than last sent block number. This means the chain re-orged!");
            panic!("Block number is less than last sent block number. This means the chain re-orged!");
        }

        if block_number != last_sent_block_number {
            for unsent_block_number in (last_sent_block_number + 1)..=block_number {
                last_sent_block_number = unsent_block_number;
                let block = evm.get_maybe_sealed_block(unsent_block_number, state).unwrap_or_else(|| panic!("The impossible happend: failed to get sealed block by number for a block in range. Block number: {unsent_block_number}. Current range: {all_block_numbers:?}"));
                let hash = block.hash().unwrap_or_default();
                let header = Sealed::new_unchecked(block.header().clone(), hash);
                let header = Header::from_consensus(header, None, None);

                let msg = SubscriptionMessage::new(
                    accepted_sink.method_name(),
                    accepted_sink.subscription_id(),
                    &header,
                )
                .unwrap_or_else(|err| {
                    panic!("Impossible: can't serialize header. Header: {header:?}, Err: {err:?}",)
                });

                if let Err(err) = accepted_sink.send(msg).await {
                    tracing::info!(%err, "The subscription client disconnected from the server.");
                    return;
                }

            }
        }

    }
}

#[derive(Error, Debug)]
enum ParamsValidationError {
    #[error("Block Option parameters are not supported in LOG subscriptions. Please use eth_getLogs or eth_getLogsWithCursor")]
    BlockOptionParam,
    #[error("Boolean parameters are not supported in LOG subscriptions")]
    BoolParam,
    #[error("Only LOG or NEW_HEADS subscriptions are supported")]
    OnlyLogOrNewHeadsSubscription,
    #[error("No paramaters are supported for NEW_HEADS subscriptions")]
    ParamsForNewHeadsSubscription,
}

enum SupportedSubscriptionParams {
    Logs(Box<Filter>),
    NewHeads,
}

fn validate_params_subscription(
    kind: SubscriptionKind,
    params: Params,
) -> Result<SupportedSubscriptionParams, ParamsValidationError> {
    match kind {
        SubscriptionKind::Logs =>{
            let filter = match params {
                Params::Logs(filter) => {
                    if filter.block_option == FilterBlockOption::default() {
                        filter
                    } else {
                        return Err(ParamsValidationError::BlockOptionParam)
                    }
                }
                Params::Bool(_) => return Err(ParamsValidationError::BoolParam),
                Params::None => Default::default(),
            };
            return Ok(SupportedSubscriptionParams::Logs(filter));
        }
        SubscriptionKind::NewHeads => {
            if params != Params::None {
                return Err(ParamsValidationError::ParamsForNewHeadsSubscription);
            }
            return Ok(SupportedSubscriptionParams::NewHeads);
        }
        _ => {
            return Err(ParamsValidationError::OnlyLogOrNewHeadsSubscription);
        }
    }

   
}
