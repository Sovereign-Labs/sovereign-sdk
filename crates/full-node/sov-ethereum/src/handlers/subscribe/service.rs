use super::watermark::Watermark;
use crate::Ethereum;
use alloy_rpc_types::Filter;
use jsonrpsee::DisconnectError;
use jsonrpsee::SubscriptionMessage;
use jsonrpsee::SubscriptionSink;
use serde::Serialize;
use sov_address::{EthereumAddress, FromVmAddress};
pub use sov_evm::EthereumAuthenticator;
use sov_evm::Evm;
use sov_evm::MaybeSealedBlock;
use sov_evm::Receipt;
use sov_modules_api::capabilities::HasKernel;
use sov_modules_api::da::Time;
use sov_modules_api::ApiStateAccessor;
use sov_modules_api::Spec;
use sov_rpc_eth_types::EthApiError;
use sov_rpc_eth_types::LogWithExecutionTimestamp;
use sov_sequencer::Sequencer;
use std::fmt::Debug;
use std::sync::Arc;
use thiserror::Error;

/// Don't send new heads notifcations for synthetic blocks more frequently than this.
const SYNTHETIC_NEW_HEADS_MAX_FREQUENCY_MS: u64 = 200;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Block does not exist")]
    BlockDoesNotExist,
    #[error("Receipt does not exist")]
    ReceiptDoesNotExist,
    #[error(transparent)]
    EthApi(#[from] EthApiError),
    #[error(transparent)]
    Disconnect(#[from] DisconnectError),
}

pub struct Streamer<S, Seq>
where
    S: Spec,
    Seq: Sequencer<Spec = S>,
{
    sink: SubscriptionSink,
    ethereum: Arc<Ethereum<S, Seq>>,
    evm: Evm<S>,
}

struct SyntheticBlockWatermark {
    block_number_of_last_notification: u64,
    // The tx index of the last notification, if known. 
    // It's not known to use when we send a notification for a *real* block, 
    // so we have special handling in that case
    tx_index_of_last_notification_if_known: Option<u64>,
}

enum SyntheticBlockWatermarkAdvanceResult {
    NewRealBlock(u64),
    NewSyntheticBlock,
    NoChange,
}

impl SyntheticBlockWatermark {
    fn from_synthetic_block(synthetic_block: &SyntheticBlockWithoutRootsAndBloom) -> Self {
        Self {
            block_number_of_last_notification: synthetic_block.header.number,
            tx_index_of_last_notification_if_known: synthetic_block.transactions.end,
        }
    }

    fn advance_and_emit_synthetic_block_notification(&mut self, synthetic_block: &SyntheticBlockWithoutRootsAndBloom) -> SyntheticBlockWatermarkAdvanceResult {
        self.block_number_of_last_notification = synthetic_block.header.number;
        self.tx_index_of_last_notification_if_known = synthetic_block.transactions.end.saturating_sub(1);
        SyntheticBlockWatermarkAdvanceResult::NewSyntheticBlock
    }


    fn advance_and_emit_real_block_notification(&mut self) -> SyntheticBlockWatermarkAdvanceResult {
        // Per https://www.quicknode.com/docs/ethereum/eth_subscribe - we should send a notification each time a new header is appended
        // Even if that means sending multiple notifications in succession. This means that we should increment the block number by one
        // rather than jumping to the height of the synthetic block.
        self.block_number_of_last_notification += 1;
        // There's no guaranteed relationship between the first tx index of the synthetic block and the last tx index of the *current* real block that we need to notify for.
        // We might have missed some synthetic block notifications due to tokio's nondeterminism. That's fine; just set it to None to reflect that we don't know for sure.
        self.tx_index_of_last_notification_if_known = None;
        SyntheticBlockWatermarkAdvanceResult::NewRealBlock(self.block_number_of_last_notification)
    }
    

    // We want to send a notification if...
    // There's been a new real block
    // There's been a new synthetic block *and* it's been more than 200ms since the last notification
    fn advance(&mut self, synthetic_block: &SyntheticBlockWithoutRootsAndBloom) -> SyntheticBlockWatermarkAdvanceResult {
        if self.block_number_of_last_notification < synthetic_block.header.number {
            return self.advance_and_emit_real_block_notification();
        }

        // If the synthetic block is empty, don't notify. 
        // Similarly if we've already sent a notification for this synthetic block, don't notify again.
        if synthetic_block.num_transactions() != 0 && || self.tx_index_of_last_notification_if_known.unwrap_or(0) < synthetic_block.last_tx_index() {
            return self.advance_and_emit_synthetic_block_notification(synthetic_block);
        }

        SyntheticBlockWatermarkAdvanceResult::NoChange
    }

}

// TODO: Refactor this into a long-running background task to reduce overhead. Right now, we do duplicate fetching for each subscription.
impl<S, Seq> Streamer<S, Seq>
where
    S: Spec,
    Seq: Sequencer<Spec = S>,
    S::Address: FromVmAddress<EthereumAddress>,
    Seq::Rt: HasKernel<S> + EthereumAuthenticator<S> + Default + Send + Sync + 'static,
{
    pub fn new(sink: SubscriptionSink, ethereum: Arc<Ethereum<S, Seq>>) -> Self {
        Self {
            sink,
            ethereum,
            evm: Default::default(),
        }
    }

    /// Stream new logs matching the provided filter to the subscriber.
    pub async fn logs(&self, filter: Box<Filter>) -> Result<(), Error> {
        let mut state = self.ethereum.api_state_accessor();
        let pending_block = self.evm.pending_block(None, &mut state);
        let synthetic_block_watermark = SyntheticBlockWatermark::from_synthetic_block(pending_block);
        let mut tx_watermark = Watermark::new(..pending_block.transactions.end);

        // Fetch the initial block. If it's stale, it will be replaced below.
        let mut block = self.get_block(pending_block.header.number - 1, &mut state)?;

        let mut state_updates = self.ethereum.sequencer.api_state().checkpoint_receiver();
        let mut shutdown_receiver = self.ethereum.shutdown_receiver.clone();

        loop {
            tokio::select! {
                result = state_updates.changed() => {
                    if result.is_err() {
                        tracing::debug!("Subscription state updates channel closed, terminating logs subscription");
                        break;
                    }

                    let mut state = self.ethereum.api_state_accessor();
                    let pending_block = self.evm.pending_block(None, &mut state);

                    for tx_idx in tx_watermark.advance(..pending_block.transactions.end) {
                        let (receipt, time) = self.get_receipt(tx_idx, &mut state)?;

                        if block.number() != receipt.block_number {
                            block = self.get_block(receipt.block_number, &mut state)?;
                        }

                        self.send_matching_logs(&receipt, &block, &filter, time).await?;
                    }
                }
                _ = shutdown_receiver.changed() => {
                    tracing::info!("Shutdown signal received, terminating logs subscription gracefully");
                    break;
                }
            }
        }
        Ok(())
    }

    /// Stream new block headers to the subscriber.
    pub async fn new_heads(&self) -> Result<(), Error> {
        // Pick up here
        // Long term todo: Refactor this into a long-running background task 

        let mut state = self.ethereum.api_state_accessor();
        let pending_block = self.evm.pending_block(None, &mut state);
        let mut watermark = SyntheticBlockWatermark::from_synthetic_block(&pending_block);

        let mut state_updates = self.ethereum.sequencer.api_state().checkpoint_receiver();
        let mut shutdown_receiver = self.ethereum.shutdown_receiver.clone();

        let mut last_send_time = Instant::now();
        loop {
            tokio::select! {
                result = state_updates.changed() => {
                    if result.is_err() {
                        tracing::debug!("Subscription state updates channel closed, terminating blocks subscription");
                        break;
                    }

                    // TODO(@preston-evans98) - I think this checkpoint change notification might be problematic on resync/restart. Figure that out.
                    // TODO: Wait - if it's been less than 200ms since the last update, skip the pending block check.
                    let mut state = self.ethereum.api_state_accessor();
                    let latest_block = self.evm.latest_block(&mut state);

                    let mut sent = false;
                    for block_number in block_watermark.advance(..latest_block.header.number + 1) {
                        let block = self.get_block(block_number, &mut state)?;
                        let rpc_header = self.evm.get_rpc_header(block, &mut state)?;
                        sent = true;
                        self.send(&rpc_header).await?;
                    }
                    if sent {
                        last_update_time = Instant::now();
                    }
                }
                _ = shutdown_receiver.changed() => {
                    tracing::info!("Shutdown signal received, terminating blocks subscription gracefully");
                    break;
                }
            }
        }
        Ok(())
    }
}

// Helper methods
impl<S, Seq> Streamer<S, Seq>
where
    S: Spec,
    Seq: Sequencer<Spec = S>,
    S::Address: FromVmAddress<EthereumAddress>,
    Seq::Rt: HasKernel<S> + EthereumAuthenticator<S> + Default + Send + Sync + 'static,
{
    fn get_block(
        &self,
        number: u64,
        state: &mut ApiStateAccessor<S>,
    ) -> Result<MaybeSealedBlock, Error> {
        self.evm
            .get_maybe_sealed_block(number, state)
            .ok_or(Error::BlockDoesNotExist)
            .inspect_err(|_| tracing::error!(number, "Block does not exist"))
    }

    fn get_receipt(
        &self,
        idx: u64,
        state: &mut ApiStateAccessor<S>,
    ) -> Result<(Receipt, Time), Error> {
        self.evm
            .receipt(idx, state)
            .ok_or(Error::ReceiptDoesNotExist)
            .inspect_err(|_| tracing::error!(idx, "Receipt does not exist"))
    }

    async fn send_matching_logs(
        &self,
        receipt: &Receipt,
        block: &MaybeSealedBlock,
        filter: &Filter,
        time: Time,
    ) -> Result<(), DisconnectError> {
        for (log_index_in_tx, log) in receipt.receipt.logs.iter().enumerate() {
            if filter.matches(log) {
                let rpc_log = LogWithExecutionTimestamp {
                    log: alloy_rpc_types::Log {
                        inner: log.clone(),
                        block_hash: block.hash(),
                        block_number: Some(block.number()),
                        block_timestamp: Some(block.timestamp()),
                        transaction_hash: Some(receipt.transaction_hash),
                        transaction_index: Some(receipt.transaction_index),
                        log_index: Some(receipt.log_index_start + log_index_in_tx as u64),
                        removed: false,
                    },
                    time_executed_ms: time.as_millis().try_into().unwrap_or_default(),
                };
                self.send(&rpc_log).await?;
            }
        }
        Ok(())
    }

    async fn send<T: Serialize + Debug>(&self, data: &T) -> Result<(), DisconnectError> {
        let msg =
            SubscriptionMessage::new(self.sink.method_name(), self.sink.subscription_id(), data)
                .expect("Failed to serialize subscription message");

        self.sink.send(msg).await.inspect_err(|err| {
            tracing::debug!(%err, "The subscription client disconnected from the server.");
        })
    }
}
