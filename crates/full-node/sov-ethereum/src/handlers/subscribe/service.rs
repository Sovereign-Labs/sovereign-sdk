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
use sov_modules_api::ApiStateAccessor;
use sov_modules_api::Spec;
use sov_sequencer::Sequencer;
use std::fmt::Debug;
use std::sync::Arc;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Block does not eist")]
    BlockDoesNotExist,
    #[error("Receipt does not eist")]
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

    fn get_receipt(&self, idx: u64, state: &mut ApiStateAccessor<S>) -> Result<Receipt, Error> {
        self.evm
            .receipt(idx, state)
            .ok_or(Error::ReceiptDoesNotExist)
            .inspect_err(|_| tracing::error!(idx, "Receipt does not exist"))
    }

    pub async fn logs(&self, filter: Box<Filter>) -> Result<(), Error> {
        let mut state = self.ethereum.api_state_accessor();

        let pending_block = self.evm.pending_block(&mut state);
        let mut tx_watermark = Watermark::new(pending_block.transactions.end);

        // Fetch the initial block. If it's stale, it will be replaced below.
        let mut block = self.get_block(pending_block.header.number - 1, &mut state)?;

        let mut state_updates = self.ethereum.sequencer.api_state().checkpoint_receiver();

        while state_updates.changed().await.is_ok() {
            let mut state = self.ethereum.api_state_accessor();

            let pending_block = self.evm.pending_block(&mut state);

            for index in tx_watermark.advance(pending_block.transactions.end) {
                let receipt = self.get_receipt(index, &mut state)?;

                if block.number() != receipt.block_number {
                    let new_block = self.get_block(receipt.block_number, &mut state)?;
                    block = new_block;
                }

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

                        self.send(&rpc_log, "log").await?;
                    }
                }
            }
        }
        Ok(())
    }

    pub async fn blocks(&self) -> Result<(), Error> {
        let mut state = self.ethereum.api_state_accessor();

        let pending_block = self.evm.pending_block(&mut state);
        let mut block_watermark = Watermark::new(pending_block.header.number - 1);

        let mut state_updates = self.ethereum.sequencer.api_state().checkpoint_receiver();
        while state_updates.changed().await.is_ok() {
            let mut state = self.ethereum.api_state_accessor();

            let pending_block = self.evm.pending_block(&mut state);

            for block_number in block_watermark.advance(pending_block.header.number - 1) {
                let block = self.get_block(block_number, &mut state)?;
                let hash = block.hash().unwrap_or_default();
                let header = Sealed::new_unchecked(block.header().clone(), hash);
                let rpc_header = Header::from_consensus(header, None, None);

                self.send(&rpc_header, "header").await?;
            }
        }
        Ok(())
    }

    async fn send<T: Serialize + Debug>(
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
