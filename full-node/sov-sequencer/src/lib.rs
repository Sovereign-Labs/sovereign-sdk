#![deny(missing_docs)]
#![doc = include_str!("../README.md")]
use std::fmt::Display;
use std::hash::Hash;
use std::str::FromStr;

/// Concrete implementations of `[BatchBuilder]`
pub mod batch_builder;
mod rpc;
/// Utilities for the sequencer rpc
pub mod utils;

use anyhow::anyhow;
use jsonrpsee::core::StringError;
use jsonrpsee::{PendingSubscriptionSink, SubscriptionMessage};
use mini_moka::sync::Cache as MokaCache;
use sov_rollup_interface::services::batch_builder::BatchBuilder;
use sov_rollup_interface::services::da::DaService;
use tokio::sync::broadcast;
use tokio::sync::Mutex;

type TxHash = [u8; 32];

const MOKA_DEFAULT_MAX_CAPACITY: u64 = 100;

/// Single data structure that manages mempool and batch producing.
pub struct Sequencer<B: BatchBuilder, Da: DaService> {
    tx_status_updates_sender: broadcast::Sender<TxStatusUpdate<B::TxHash, Da>>,
    tx_status_updates_receiver: broadcast::Receiver<TxStatusUpdate<B::TxHash, Da>>,
    tx_statuses_cache: MokaCache<B::TxHash, TxStatus<Da>>,
    batch_builder: Mutex<B>,
    da_service: Da,
}

impl<B, Da> Sequencer<B, Da>
where
    B: BatchBuilder + Send + Sync,
    B::TxHash: Hash + Eq + Clone + FromStr + Send + Sync + 'static,
    <B::TxHash as FromStr>::Err: Display,
    Da: DaService + Send + Sync,
{
    /// Creates new Sequencer from BatchBuilder and DaService
    pub fn new(batch_builder: B, da_service: Da) -> Self {
        let tx_statuses_cache = MokaCache::new(MOKA_DEFAULT_MAX_CAPACITY);
        let tx_statuses_cache_clone = tx_statuses_cache.clone();

        let (tx_status_updates_sender, tx_status_updates_receiver) = broadcast::channel(100);
        let mut recv = tx_status_updates_receiver.resubscribe();
        tokio::spawn(async move {
            while let Ok(TxStatusUpdate { tx_hash, status }) = recv.recv().await {
                tx_statuses_cache_clone.insert(tx_hash, status);
            }
        });

        Self {
            tx_status_updates_sender,
            tx_status_updates_receiver,
            tx_statuses_cache: MokaCache::new(MOKA_DEFAULT_MAX_CAPACITY),
            batch_builder: Mutex::new(batch_builder),
            da_service,
        }
    }

    async fn submit_batch(&self) -> anyhow::Result<usize> {
        // Need to release lock before await, so the Future is `Send`.
        // But potentially it can create blobs that are sent out of order.
        // It can be improved with atomics,
        // so a new batch is only created after previous was submitted.
        tracing::info!("Submit batch request has been received!");
        let blob = {
            let mut batch_builder = self.batch_builder.lock().await;
            batch_builder.get_next_blob()?
        };

        let num_txs = blob.len();
        let (blob, tx_hashes) = blob
            .into_iter()
            .map(|tx| (tx.raw_tx, tx.hash))
            .unzip::<_, _, Vec<_>, Vec<_>>();
        let blob = borsh::to_vec(&blob)?;
        for tx_hash in tx_hashes {
            self.tx_status_updates_sender
                .send(TxStatusUpdate {
                    tx_hash,
                    status: TxStatus::Submitted,
                })
                .map_err(|e| anyhow!("failed to send tx status update: {}", e.to_string()))?;
        }

        match self.da_service.send_transaction(&blob).await {
            Ok(_) => Ok(num_txs),
            Err(e) => Err(anyhow!("failed to submit batch: {:?}", e)),
        }
    }

    async fn accept_tx(&self, tx: Vec<u8>) -> anyhow::Result<()> {
        tracing::info!("Accepting tx: 0x{}", hex::encode(&tx));
        let mut batch_builder = self.batch_builder.lock().await;
        batch_builder.accept_tx(tx)?;
        Ok(())
    }

    async fn handle_tx_status_update_subscription(
        &self,
        params: jsonrpsee::types::Params<'_>,
        sink: PendingSubscriptionSink,
    ) -> Result<(), StringError> {
        let tx_hash_str: String = params.one()?;
        let tx_hash = B::TxHash::from_str(&tx_hash_str)?;

        let mut receiver = self.tx_status_updates_receiver.resubscribe();
        let subscription = sink.accept().await?;
        while let Ok(TxStatusUpdate {
            tx_hash: txh,
            status,
        }) = receiver.recv().await
        {
            // We're only interested in updates for the requested transaction hash.
            if tx_hash != txh {
                continue;
            }

            let notification = SubscriptionMessage::from_json(&status)?;
            subscription.send(notification).await?;
        }

        Ok(())
    }
}

#[derive(Debug, PartialEq, Eq)]
struct TxStatusUpdate<Hash, Da: DaService> {
    tx_hash: Hash,
    status: TxStatus<Da>,
}

impl<Hash, Da: DaService> Clone for TxStatusUpdate<Hash, Da> {
    fn clone(&self) -> Self {
        Self {
            tx_hash: self.tx_hash.clone(),
            status: self.status.clone(),
        }
    }
}

/// Hex string
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct HexString(pub [u8; 32]);

impl FromStr for HexString {
    type Err = hex::FromHexError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.strip_prefix("0x").unwrap_or(s);
        let bytes = hex::decode(s)?;
        if bytes.len() != 32 {
            return Err(hex::FromHexError::InvalidStringLength);
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        Ok(HexString(arr))
    }
}

impl ToString for HexString {
    fn to_string(&self) -> String {
        format!("0x{}", hex::encode(&self.0))
    }
}

/// A transaction to be submitted to the rollup
#[derive(serde::Serialize, serde::Deserialize)]
pub struct SubmitTransaction {
    body: Vec<u8>,
}

impl SubmitTransaction {
    /// Creates a new transaction for submission to the rollup
    pub fn new(body: Vec<u8>) -> Self {
        SubmitTransaction { body }
    }
}

/// The result of submitting a transaction to the rollup
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum SubmitTransactionResponse {
    /// Submission succeeded
    Registered,
    /// Submission failed with given reason
    Failed(String),
}

/// A rollup transaction status.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TxStatus<D: DaService> {
    /// The transaction was successfully submitted to a sequencer and it's
    /// sitting in the mempool waiting to be included in a batch.
    Submitted,
    /// The transaction was published to the DA as part of a batch, but it may
    /// not be finalized yet.
    Published { da_transaction_id: D::TransactionId },
    /// The transaction was published to the DA as part of a batch that is
    /// considered finalized
    Finalized,
}

#[cfg(test)]
mod tests {

    use sov_mock_da::{MockAddress, MockDaService};
    use sov_rollup_interface::{da::BlobReaderTrait, services::batch_builder::TxWithHash};

    use crate::rpc::get_sequencer_rpc;

    use super::*;

    /// BatchBuilder used in tests.
    pub struct MockBatchBuilder {
        /// Mempool with transactions.
        pub mempool: Vec<Vec<u8>>,
    }

    // It only takes the first byte of the tx, when submits it.
    // This allows to show effect of batch builder
    impl BatchBuilder for MockBatchBuilder {
        type TxHash = HexString;

        fn accept_tx(&mut self, tx: Vec<u8>) -> anyhow::Result<Self::TxHash> {
            self.mempool.push(tx);
            Ok(HexString([0; 32]))
        }

        fn contains(&self, _tx_hash: &Self::TxHash) -> bool {
            unimplemented!("MockBatchBuilder::contains is not implemented")
        }

        fn get_next_blob(&mut self) -> anyhow::Result<Vec<TxWithHash<Self::TxHash>>> {
            if self.mempool.is_empty() {
                anyhow::bail!("Mock mempool is empty");
            }
            let txs = std::mem::take(&mut self.mempool)
                .into_iter()
                .filter_map(|tx| {
                    let first_byte = *tx.get(0)?;
                    Some(TxWithHash {
                        raw_tx: vec![first_byte],
                        hash: HexString([0; 32]),
                    })
                })
                .collect();
            Ok(txs)
        }
    }

    #[tokio::test]
    async fn test_submit_on_empty_mempool() {
        let batch_builder = MockBatchBuilder { mempool: vec![] };
        let da_service = MockDaService::new(MockAddress::default());
        let rpc = get_sequencer_rpc(batch_builder, da_service.clone());

        let arg: &[u8] = &[];
        let result: Result<String, jsonrpsee::core::Error> =
            rpc.call("sequencer_publishBatch", arg).await;

        assert!(result.is_err());
        let error = result.err().unwrap();
        assert_eq!(
            "ErrorObject { code: ServerError(-32001), message: \"SEQUENCER_RPC_ERROR\", data: Some(RawValue(\"Mock mempool is empty\")) }",
            error.to_string()
        );
    }

    #[tokio::test]
    async fn test_submit_happy_path() {
        let tx1 = vec![1, 2, 3];
        let tx2 = vec![3, 4, 5];
        let batch_builder = MockBatchBuilder {
            mempool: vec![tx1.clone(), tx2.clone()],
        };
        let da_service = MockDaService::new(MockAddress::default());
        let rpc = get_sequencer_rpc(batch_builder, da_service.clone());

        let arg: &[u8] = &[];
        let _: String = rpc.call("sequencer_publishBatch", arg).await.unwrap();

        let mut submitted_block = da_service.get_block_at(1).await.unwrap();
        let block_data = submitted_block.blobs[0].full_data();

        // First bytes of each tx, flattened
        let blob: Vec<Vec<u8>> = vec![vec![tx1[0]], vec![tx2[0]]];
        let expected: Vec<u8> = borsh::to_vec(&blob).unwrap();
        assert_eq!(expected, block_data);
    }

    #[tokio::test]
    async fn test_accept_tx() {
        let batch_builder = MockBatchBuilder { mempool: vec![] };
        let da_service = MockDaService::new(MockAddress::default());

        let rpc = get_sequencer_rpc(batch_builder, da_service.clone());

        let tx: Vec<u8> = vec![1, 2, 3, 4, 5];
        let request = SubmitTransaction { body: tx.clone() };
        let result: SubmitTransactionResponse =
            rpc.call("sequencer_acceptTx", [request]).await.unwrap();
        assert_eq!(SubmitTransactionResponse::Registered, result);

        let arg: &[u8] = &[];
        let _: String = rpc.call("sequencer_publishBatch", arg).await.unwrap();

        let mut submitted_block = da_service.get_block_at(1).await.unwrap();
        let block_data = submitted_block.blobs[0].full_data();

        // First bytes of each tx, flattened
        let blob: Vec<Vec<u8>> = vec![vec![tx[0]]];
        let expected: Vec<u8> = borsh::to_vec(&blob).unwrap();
        assert_eq!(expected, block_data);
    }

    #[tokio::test]
    #[ignore = "TBD"]
    async fn test_full_flow() {}
}
