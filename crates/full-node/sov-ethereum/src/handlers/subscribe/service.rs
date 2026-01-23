use super::watermark::Watermark;
use crate::Ethereum;
use alloy_primitives::U256;
use alloy_rpc_types::{Filter, Header};
use jsonrpsee::DisconnectError;
use jsonrpsee::SubscriptionMessage;
use jsonrpsee::SubscriptionSink;
use serde::Serialize;
use sov_address::{EthereumAddress, FromVmAddress};
pub use sov_evm::EthereumAuthenticator;
use sov_evm::Evm;
use sov_evm::MaybeSealedBlock;
use sov_evm::Receipt;
use sov_evm::SyntheticBlockWithoutRootsAndBloom;
use sov_modules_api::capabilities::HasKernel;
use sov_modules_api::da::Time;
use sov_modules_api::ApiStateAccessor;
use sov_modules_api::Spec;
use sov_modules_api::prelude::UnwrapInfallible;
use sov_rpc_eth_types::EthApiError;
use sov_rpc_eth_types::LogWithExecutionTimestamp;
use sov_sequencer::Sequencer;
use std::time::Duration;
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

#[derive(Debug, Clone)]
struct SyntheticBlockWatermark {
    block_number_of_last_notification: u64,
    // The tx index of the last notification, if known.
    // It's not known to use when we send a notification for a *real* block,
    // so we have special handling in that case
    tx_index_of_last_notification_if_known: Option<u64>,
}

#[derive(Debug, PartialEq, Clone)]
enum SyntheticBlockWatermarkAdvanceResult {
    NewRealBlock(u64),
    NewSyntheticBlock,
    NoChange,
}

impl SyntheticBlockWatermark {
    fn from_synthetic_block(synthetic_block: &SyntheticBlockWithoutRootsAndBloom) -> Self {
        Self {
            block_number_of_last_notification: synthetic_block.header().number,
            tx_index_of_last_notification_if_known: Some(synthetic_block.last_tx_index()),
        }
    }

    fn advance_and_emit_synthetic_block_notification(&mut self, synthetic_block: &SyntheticBlockWithoutRootsAndBloom) -> SyntheticBlockWatermarkAdvanceResult {
        self.block_number_of_last_notification = synthetic_block.header().number;
        self.tx_index_of_last_notification_if_known = Some(synthetic_block.last_tx_index());
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
        if self.block_number_of_last_notification < synthetic_block.header().number.saturating_sub(1) {
            return self.advance_and_emit_real_block_notification();
        }

        // If the synthetic block is empty, don't notify.
        // Similarly if we've already sent a notification for this synthetic block, don't notify again.
        if synthetic_block.num_transactions() != 0 && self.tx_index_of_last_notification_if_known.map_or(true, |idx| idx < synthetic_block.last_tx_index()) {
            return self.advance_and_emit_synthetic_block_notification(synthetic_block);
        }

        SyntheticBlockWatermarkAdvanceResult::NoChange
    }

    fn peek(&self, synthetic_block: &SyntheticBlockWithoutRootsAndBloom) -> SyntheticBlockWatermarkAdvanceResult {
        if self.block_number_of_last_notification < synthetic_block.header().number.saturating_sub(1) {
            return SyntheticBlockWatermarkAdvanceResult::NewRealBlock(self.block_number_of_last_notification + 1)
        }

        if synthetic_block.num_transactions() != 0 && self.tx_index_of_last_notification_if_known.map_or(true, |idx| idx < synthetic_block.last_tx_index()) {
            return SyntheticBlockWatermarkAdvanceResult::NewSyntheticBlock
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
        let mut tx_watermark = Watermark::new(..pending_block.transactions.end);

        // Fetch the initial block. If it's stale, it will be replaced below.
        let mut block = self.get_block(pending_block.block_number() - 1, &mut state)?;

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
    /// 
    /// This method streams every real block as it comes in (i.e. each time a "slot" is computed on the DA layer). 
    /// We also manufacture "synthetic" blocks with which to notify the subscriber each time a new transaction is added,
    /// but we only notify for synthetic blocks at most once every few hundred milliseconds. 
    /// 
    /// We use synthetic blocks because...
    /// - Rollup transactions are instantly confirmed 
    /// - Most wallet software and tooling waits for the tx to be included in the "latest" (i.e. non-pending)
    /// - We want tooling to recognize that transactions are confirmed instantly. 
    /// 
    /// By making up these sythetic blocks, we can simulate behavior where the tx is accepted instantly and then the chain experiences a reorg of depth 1.
    /// Tooling that *doesn't* handle reorgs works fine, because the "reorg" simply appends new transactions - so previous tx results are unchanged.
    /// Tolling that *does* handle reorgs will see breif instability at the chain head, but the block will settle after the next DA block is computed (i.e in about 6 seconds)
    /// 
    /// Unfortunately, this approach means that we have a *lot* of new_heads notifications (one per tx, plus one per DA block. We expect that receivers will not be able
    /// to cope with notifications at a pace of several hundred per second, so we impose a throttle - we never notify more than once per 200ms. 
    pub async fn new_heads(&self) -> Result<(), Error> {
        // Pick up here
        // Long term todo: Refactor this into a long-running background task 

        let mut state = self.ethereum.api_state_accessor();
        let mut watermark = SyntheticBlockWatermark::from_synthetic_block(&self.evm.pending_block(None, &mut state));

        let mut state_updates = self.ethereum.sequencer.api_state().checkpoint_receiver();
        let mut shutdown_receiver = self.ethereum.shutdown_receiver.clone();
        let mut last_send_time = std::time::Instant::now();
        let (wakeup_sender, mut wakeup_receiver) = tokio::sync::watch::channel(());
        loop {
            tokio::select! {
                // Case 1: we just received a new state update. Notify for any new blocks
                result = state_updates.changed() => {
                    if result.is_err() {
                        tracing::debug!("Subscription state updates channel closed, terminating blocks subscription");
                        break;
                    }

                    let mut state = self.ethereum.api_state_accessor();
                    let pending_block = self.evm.pending_block(None, &mut state);
                    let mut sent = false;

                    // Send all of the notifications for new real blocks.
                    while let SyntheticBlockWatermarkAdvanceResult::NewRealBlock(block_number) = watermark.peek(&pending_block) {
                        watermark.advance(&pending_block);
                        let sealed = self.evm.blocks.get(&block_number, &mut state).unwrap_infallible().expect("Block was notified but did not exist. This is a bug!");
                        let rpc_header = Header::from_consensus(sealed.header.into(), None, Some(U256::from(sealed.rlp_size)));
                        self.send(&rpc_header).await?;
                        sent = true;
                    }

                    // If the state change only created a synthetic block, send it only if we haven't sent one too recently
                    if let SyntheticBlockWatermarkAdvanceResult::NewSyntheticBlock = watermark.peek(&pending_block) {
                        if last_send_time.elapsed() >= Duration::from_millis(SYNTHETIC_NEW_HEADS_MAX_FREQUENCY_MS) {
                            watermark.advance(&pending_block);
                            let (rpc_header, _txs) = self.evm.get_synthetic_block_contents_slow(pending_block, &mut state)?;
                            self.send(&rpc_header).await?;
                            sent = true;
                        } else {
                            // If we don't send the notification now, scheudle a wakeup to try again later. This handles the edge case
                            // where we have a bunch of new synethitic blocks in rapid succession followed by a gap; in that case, 
                            // we'd never notify for the newer txs.
                            // Spawn a wakeup for later to handle the edge case. Note that we don't try to dedup wakeups; duplicates are handled in the wakeup_receiver case.
                            let wakeup_sender = wakeup_sender.clone();
                            tokio::spawn(async move {
                                tokio::time::sleep(Duration::from_millis(SYNTHETIC_NEW_HEADS_MAX_FREQUENCY_MS) ).await;
                                let _ = wakeup_sender.send(());
                            });
                        }
                    }
                    
                    if sent {
                        last_send_time = std::time::Instant::now();
                    }
                    // In the SyntheticBlockWatermarkAdvanceResult::None case, do nothing.
                }
                _ = wakeup_receiver.changed() => {
                    // If we've sent a notification too recently, ignore the wakeup. Either we've sent all notifications already or - we'll get woken up again in no more than 200ms
                    if last_send_time.elapsed() >= Duration::from_millis(SYNTHETIC_NEW_HEADS_MAX_FREQUENCY_MS) {
                        let mut state = self.ethereum.api_state_accessor();
                        let pending_block = self.evm.pending_block(None, &mut state);
                        // Only send the notification if it's for a synthetic block. Real blocks are guaranteed to be handeld by the main state_change watcher.
                        if let SyntheticBlockWatermarkAdvanceResult::NewSyntheticBlock = watermark.peek(&pending_block) {
                            watermark.advance(&pending_block);
                            let (rpc_header, _txs) = self.evm.get_synthetic_block_contents_slow(pending_block, &mut state)?;
                            self.send(&rpc_header).await?;
                            last_send_time = std::time::Instant::now();
                        } 
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

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_consensus::Header;

    fn make_synthetic_block(block_number: u64, tx_start: u64, tx_end: u64) -> SyntheticBlockWithoutRootsAndBloom {
        let mut header = Header::default();
        header.number = block_number;
        SyntheticBlockWithoutRootsAndBloom::new(header, tx_start..tx_end)
    }

    /// Helper to verify peek and advance return the same result variant.
    fn assert_peek_equals_and_advance(
        watermark: &mut SyntheticBlockWatermark,
        block: &SyntheticBlockWithoutRootsAndBloom,
    ) -> SyntheticBlockWatermarkAdvanceResult {
        let peek_result = watermark.peek(block);
        let advance_result = watermark.advance(block);
        assert_eq!(
            peek_result, advance_result,
            "peek and advance should return the same result"
        );
        peek_result
    }
    

    #[test]
    fn peek_and_advance_match_for_new_real_block() {
        // Watermark at block 5, synthetic block is at block 7
        // Should return NewRealBlock(6) - incrementing by 1
        // Then NewSyntheticBlock
        let mut watermark = SyntheticBlockWatermark {
            block_number_of_last_notification: 4,
            tx_index_of_last_notification_if_known: Some(100),
        };
        let block = make_synthetic_block(7, 100, 150);

        assert!(matches!(assert_peek_equals_and_advance(&mut watermark, &block), SyntheticBlockWatermarkAdvanceResult::NewRealBlock(5)));
        assert!(matches!(assert_peek_equals_and_advance(&mut watermark, &block), SyntheticBlockWatermarkAdvanceResult::NewRealBlock(6)));
        assert!(matches!(assert_peek_equals_and_advance(&mut watermark, &block), SyntheticBlockWatermarkAdvanceResult::NewSyntheticBlock));
        assert!(matches!(assert_peek_equals_and_advance(&mut watermark, &block), SyntheticBlockWatermarkAdvanceResult::NoChange));
    }

    #[test]
    fn peek_and_advance_match_for_new_synthetic_block() {
        // Watermark and block at same block number, but block has newer transactions
        let mut watermark = SyntheticBlockWatermark {
            block_number_of_last_notification: 10,
            tx_index_of_last_notification_if_known: Some(50),
        };
        let block = make_synthetic_block(10, 0, 100); // last_tx_index is 99, > 50

        assert!(matches!(assert_peek_equals_and_advance(&mut watermark, &block), SyntheticBlockWatermarkAdvanceResult::NewSyntheticBlock));
        assert!(matches!(assert_peek_equals_and_advance(&mut watermark, &block), SyntheticBlockWatermarkAdvanceResult::NoChange));
    }

    #[test]
    fn peek_and_advance_match_for_no_change() {
        // Watermark already caught up with the block
        let mut watermark = SyntheticBlockWatermark {
            block_number_of_last_notification: 10,
            tx_index_of_last_notification_if_known: Some(99),
        };
        let block = make_synthetic_block(10, 0, 100); // last_tx_index is 99, matches watermark

        assert!(matches!(assert_peek_equals_and_advance(&mut watermark, &block), SyntheticBlockWatermarkAdvanceResult::NoChange));
    }

    #[test]
    fn peek_and_advance_match_for_empty_block() {
        // Block with no transactions should result in NoChange
        let mut watermark = SyntheticBlockWatermark {
            block_number_of_last_notification: 10,
            tx_index_of_last_notification_if_known: None,
        };
        let block = make_synthetic_block(10, 0, 0); // empty block

        assert!(matches!(assert_peek_equals_and_advance(&mut watermark, &block), SyntheticBlockWatermarkAdvanceResult::NoChange));
    }

    #[test]
    fn peek_and_advance_match_when_tx_index_unknown() {
        // When tx_index_of_last_notification_if_known is None, should trigger NewSyntheticBlock
        // if there are transactions
        let mut watermark = SyntheticBlockWatermark {
            block_number_of_last_notification: 10,
            tx_index_of_last_notification_if_known: None,
        };
        let block = make_synthetic_block(10, 0, 50);

        assert!(matches!(assert_peek_equals_and_advance(&mut watermark, &block), SyntheticBlockWatermarkAdvanceResult::NewSyntheticBlock));
    }
    #[test]
    fn from_synthetic_block_initializes_correctly() {
        let block = make_synthetic_block(10, 50, 100);
        let watermark = SyntheticBlockWatermark::from_synthetic_block(&block);

        assert_eq!(watermark.block_number_of_last_notification, 10);
        assert_eq!(watermark.tx_index_of_last_notification_if_known, Some(99)); // last_tx_index = end - 1
    }
}
