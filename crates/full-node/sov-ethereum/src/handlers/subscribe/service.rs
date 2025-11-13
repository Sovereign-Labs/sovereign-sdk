use super::watermark::Watermark;
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
use sov_evm::MaybeSealedBlock;
use sov_evm::Receipt;
use sov_modules_api::capabilities::HasKernel;
use sov_modules_api::da::Time;
use sov_modules_api::ApiStateAccessor;
use sov_modules_api::Spec;
use sov_rpc_eth_types::LogWithExecutionTimestamp;
use sov_sequencer::Sequencer;
use std::fmt::Debug;
use std::sync::Arc;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Block does not exist")]
    BlockDoesNotExist,
    #[error("Receipt does not exist")]
    ReceiptDoesNotExist,
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
        let pending_block = self.evm.pending_block(&mut state);
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
                    let pending_block = self.evm.pending_block(&mut state);

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
    pub async fn blocks(&self) -> Result<(), Error> {
        let mut state = self.ethereum.api_state_accessor();
        let pending_block = self.evm.pending_block(&mut state);
        let mut block_watermark = Watermark::new(..pending_block.header.number);

        let mut state_updates = self.ethereum.sequencer.api_state().checkpoint_receiver();
        let mut shutdown_receiver = self.ethereum.shutdown_receiver.clone();

        loop {
            tokio::select! {
                result = state_updates.changed() => {
                    if result.is_err() {
                        tracing::debug!("Subscription state updates channel closed, terminating blocks subscription");
                        break;
                    }

                    let mut state = self.ethereum.api_state_accessor();
                    let pending_block = self.evm.pending_block(&mut state);

                    for block_number in block_watermark.advance(..pending_block.header.number) {
                        let block = self.get_block(block_number, &mut state)?;
                        self.send_block_header(&block).await?;
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

    async fn send_block_header(&self, block: &MaybeSealedBlock) -> Result<(), DisconnectError> {
        let hash = block.hash().unwrap_or_default();
        let header = Sealed::new_unchecked(block.header().clone(), hash);
        let rpc_header = Header::from_consensus(header, None, None);
        self.send(&rpc_header).await
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
