use crate::Ethereum;
use alloy_consensus::Sealed;
use alloy_rpc_types::Filter;
use alloy_rpc_types::Header;
use jsonrpsee::DisconnectError;
use jsonrpsee::SubscriptionMessage;
use jsonrpsee::SubscriptionSink;
use serde::Serialize;
use sov_address::{EthereumAddress, FromVmAddress};
pub use sov_evm::EthereumAuthenticator;
use sov_evm::Evm;
use sov_modules_api::capabilities::HasKernel;
use sov_modules_api::Spec;
use sov_sequencer::Sequencer;
use std::fmt::Debug;
use std::sync::Arc;
use thiserror::Error;

pub struct Streamer<S, Seq>
where
    S: Spec,
    Seq: Sequencer<Spec = S>,
{
    sink: SubscriptionSink,
    ethereum: Arc<Ethereum<S, Seq>>,
}

impl<S, Seq> Streamer<S, Seq>
where
    S: Spec,
    Seq: Sequencer<Spec = S>,
    S::Address: FromVmAddress<EthereumAddress>,
    Seq::Rt: HasKernel<S> + EthereumAuthenticator<S> + Default + Send + Sync + 'static,
{
    pub fn new(sink: SubscriptionSink, ethereum: Arc<Ethereum<S, Seq>>) -> Self {
        Self { sink, ethereum }
    }

    pub async fn logs(&self, filter: Box<Filter>) -> Result<(), Error> {
        let evm = Evm::<S>::default();
        let mut state = self.ethereum.api_state_accessor();

        let pending_block = evm.pending_block(&mut state);
        let mut prev_last_tx_index = pending_block.transactions.end;

        // Fetch the initial block. If it's stale, it will be replaced below.
        let start_block = pending_block.header.number - 1;
        let Some(mut block) = evm.get_maybe_sealed_block(start_block, &mut state) else {
            tracing::error!(start_block, "Block does not exist");
            return Err(Error::BlockDoesNotExist);
        };

        let mut state_updates = self.ethereum.sequencer.api_state().checkpoint_receiver();

        while state_updates.changed().await.is_ok() {
            let mut state = self.ethereum.api_state_accessor();

            let pending_block = evm.pending_block(&mut state);
            let curr_last_tx_index = pending_block.transactions.end;

            if curr_last_tx_index <= prev_last_tx_index {
                continue;
            }

            for index in prev_last_tx_index..curr_last_tx_index {
                let Some(receipt) = evm.receipt(index, &mut state) else {
                    // This can happen if the state was pruned.
                    tracing::error!(index, "Receipt does not exist");
                    return Err(Error::ReceiptDoesNotExist);
                };

                if block.number() != receipt.block_number {
                    match evm.get_maybe_sealed_block(receipt.block_number, &mut state) {
                        Some(b) => block = b,
                        None => {
                            tracing::error!(
                                block_number = receipt.block_number,
                                "Block does not exist"
                            );
                            return Err(Error::BlockDoesNotExist);
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

                        self.send_subscription_message(&rpc_log, "log").await?;
                    }
                }
            }
            prev_last_tx_index = curr_last_tx_index;
        }
        Ok(())
    }

    pub async fn blocks(&self) -> Result<(), Error> {
        let evm = Evm::<S>::default();
        let mut state = self.ethereum.api_state_accessor();

        let pending_block = evm.pending_block(&mut state);
        let mut prev_block_number = pending_block.header.number - 1;

        let mut state_updates = self.ethereum.sequencer.api_state().checkpoint_receiver();

        while state_updates.changed().await.is_ok() {
            let mut state = self.ethereum.api_state_accessor();

            let pending_block = evm.pending_block(&mut state);
            let current_block_number = pending_block.header.number - 1;

            // Check if there's a new sealed block
            if current_block_number <= prev_block_number {
                continue;
            }

            // Send all new blocks from prev_block_number+1 to current_block_number
            for block_number in (prev_block_number + 1)..=current_block_number {
                let Some(block) = evm.get_maybe_sealed_block(block_number, &mut state) else {
                    tracing::error!(block_number, "Block does not exist");
                    return Err(Error::BlockDoesNotExist);
                };

                let hash = block.hash().unwrap_or_default();
                let header = Sealed::new_unchecked(block.header().clone(), hash);
                let rpc_header = Header::from_consensus(header, None, None);

                self.send_subscription_message(&rpc_header, "header")
                    .await?;
            }

            prev_block_number = current_block_number;
        }
        Ok(())
    }

    /// Helper function to send a message through the subscription sink
    async fn send_subscription_message<T: Serialize + Debug>(
        &self,
        data: &T,
        data_type: &str,
    ) -> Result<(), DisconnectError> {
        let msg =
            SubscriptionMessage::new(self.sink.method_name(), self.sink.subscription_id(), data)
                .unwrap_or_else(|err| {
                    panic!("Impossible: can't serialize {data_type}. Data: {data:?}, Err: {err:?}")
                });

        self.sink.send(msg).await.inspect_err(|err| {
            tracing::info!(%err, "The subscription client disconnected from the server.");
        })
    }
}

#[derive(Error, Debug)]
pub enum ParamsValidationError {
    #[error("Block Option parameters are not supported in LOG subscriptions. Please use eth_getLogs or eth_getLogsWithCursor")]
    BlockOptionParam,
    #[error("Boolean parameters are not supported in LOG subscriptions")]
    BoolParam,
    #[error("Only LOG and newHeads subscriptions are supported")]
    OnlyLogAndNewHeadsSubscription,
    #[error("newHeads subscription does not accept parameters")]
    NewHeadsDoesNotAcceptParams,
}

#[derive(Error, Debug)]
pub enum Error {
    #[error("Block does not eist")]
    BlockDoesNotExist,
    #[error("Receipt does not eist")]
    ReceiptDoesNotExist,
    #[error(transparent)]
    Disconnect(#[from] DisconnectError),
}
