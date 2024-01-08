#![deny(missing_docs)]
#![doc = include_str!("../README.md")]
use std::hash::Hash;
use std::sync::Arc;

pub mod batch_builder;
pub mod utils;

use anyhow::anyhow;
use jsonrpsee::core::StringError;
use jsonrpsee::types::ErrorObjectOwned;
use jsonrpsee::{PendingSubscriptionSink, RpcModule, SubscriptionMessage};
use mini_moka::sync::Cache as MokaCache;
use sov_modules_api::utils::to_jsonrpsee_error_object;
use sov_rollup_interface::services::batch_builder::{BatchBuilder, TxHash};
use sov_rollup_interface::services::da::DaService;
use tokio::sync::{broadcast, Mutex};
use tracing::{info, warn};

const SEQUENCER_RPC_ERROR: &str = "SEQUENCER_RPC_ERROR";

/// Single data structure that manages mempool and batch producing.
pub struct Sequencer<B: BatchBuilder, Da: DaService> {
    batch_builder: Mutex<B>,
    da_service: Da,
    tx_statuses_cache: MokaCache<TxHash, TxStatus<Da::TransactionId>>,
    tx_statuses_sender: broadcast::Sender<TxStatusUpdate<Da::TransactionId>>,
}

impl<B, Da> Sequencer<B, Da>
where
    B: BatchBuilder + Send + Sync + 'static,
    Da: DaService + Send + Sync + 'static,
    Da::TransactionId: Clone + Send + Sync + serde::Serialize,
{
    // The cache capacity is kind of arbitrary, as long as it's big enough to
    // fit a handful of typical batches worth of transactions it won't make much
    // of a difference.
    const TX_STATUSES_CACHE_CAPACITY: u64 = 300;
    // As long as we're reasonably fast at processing transaction status updates
    // (which we are!), the channel size won't matter significantly.
    const TX_STATUSES_UPDATES_CHANNEL_CAPACITY: usize = 100;

    /// Creates new Sequencer from BatchBuilder and DaService
    pub fn new(batch_builder: B, da_service: Da) -> Self {
        let tx_statuses_cache = MokaCache::new(Self::TX_STATUSES_CACHE_CAPACITY);
        let (tx_statuses_sender, mut receiver) =
            broadcast::channel(Self::TX_STATUSES_UPDATES_CHANNEL_CAPACITY);

        let tx_statuses_cache_clone = tx_statuses_cache.clone();
        tokio::spawn(async move {
            while let Ok(TxStatusUpdate { tx_hash, status }) = receiver.recv().await {
                tx_statuses_cache_clone.insert(tx_hash, status);
            }
        });

        Self {
            batch_builder: Mutex::new(batch_builder),
            da_service,
            tx_statuses_cache,
            tx_statuses_sender,
        }
    }

    /// Returns the [`jsonrpsee::RpcModule`] for the sequencer-related RPC
    /// methods.
    pub fn rpc(self) -> RpcModule<Self> {
        let mut rpc = RpcModule::new(self);
        Self::register_txs_rpc_methods(&mut rpc).expect("Failed to register sequencer RPC methods");
        rpc
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

        let da_tx_id = match self.da_service.send_transaction(&blob).await {
            Ok(id) => id,
            Err(e) => return Err(anyhow!("failed to submit batch: {:?}", e)),
        };

        for tx_hash in tx_hashes {
            self.tx_statuses_sender
                .send(TxStatusUpdate {
                    tx_hash,
                    status: TxStatus::Published {
                        da_transaction_id: da_tx_id.clone(),
                    },
                })
                .map_err(|error| warn!(%error, "Failed to send tx status update"))
                // Batch submission shouldn't fail if notifications can't be
                // sent.
                .ok();
        }

        Ok(num_txs)
    }

    async fn accept_tx(&self, tx: Vec<u8>) -> anyhow::Result<()> {
        info!("Accepting tx: 0x{}", hex::encode(&tx));
        let mut batch_builder = self.batch_builder.lock().await;
        let tx_hash = batch_builder.accept_tx(tx)?;
        self.tx_statuses_sender
            .send(TxStatusUpdate {
                tx_hash,
                status: TxStatus::Submitted,
            })
            .map_err(|e| anyhow!("failed to send tx status update: {}", e.to_string()))?;
        Ok(())
    }

    async fn tx_status(&self, tx_hash: &TxHash) -> Option<TxStatus<Da::TransactionId>> {
        let is_in_mempool = self.batch_builder.lock().await.contains(tx_hash);

        if is_in_mempool {
            Some(TxStatus::Submitted)
        } else {
            self.tx_statuses_cache.get(tx_hash)
        }
    }

    fn register_txs_rpc_methods(rpc: &mut RpcModule<Self>) -> Result<(), jsonrpsee::core::Error> {
        rpc.register_async_method(
            "sequencer_publishBatch",
            |params, batch_builder| async move {
                let mut params_iter = params.sequence();
                while let Some(tx) = params_iter.optional_next::<Vec<u8>>()? {
                    batch_builder
                        .accept_tx(tx)
                        .await
                        .map_err(|e| to_jsonrpsee_error_object(e, SEQUENCER_RPC_ERROR))?;
                }
                let num_txs = batch_builder
                    .submit_batch()
                    .await
                    .map_err(|e| to_jsonrpsee_error_object(e, SEQUENCER_RPC_ERROR))?;

                Ok::<String, ErrorObjectOwned>(format!("Submitted {} transactions", num_txs))
            },
        )?;
        rpc.register_async_method("sequencer_acceptTx", |params, sequencer| async move {
            let tx: SubmitTransaction = params.one()?;
            let response = match sequencer.accept_tx(tx.body).await {
                Ok(()) => SubmitTransactionResponse::Registered,
                Err(e) => SubmitTransactionResponse::Failed(e.to_string()),
            };
            Ok::<_, ErrorObjectOwned>(response)
        })?;

        rpc.register_async_method("sequencer_txStatus", |params, sequencer| async move {
            let tx_hash: HexHash = params.one()?;

            let status = sequencer.tx_status(&tx_hash.0).await;
            Ok::<_, ErrorObjectOwned>(status)
        })?;
        rpc.register_subscription(
            "sequencer_subscribeToTxStatusUpdates",
            "sequencer_newTxStatus",
            "sequencer_unsubscribeToTxStatusUpdates",
            |params, pending, sequencer| async move {
                Self::handle_tx_status_update_subscription(sequencer, params, pending).await
            },
        )?;

        Ok(())
    }

    async fn handle_tx_status_update_subscription(
        sequencer: Arc<Self>,
        params: jsonrpsee::types::Params<'_>,
        sink: PendingSubscriptionSink,
    ) -> Result<(), StringError> {
        let tx_hash: HexHash = params.one()?;

        let mut receiver = sequencer.tx_statuses_sender.subscribe();
        let subscription = sink.accept().await?;
        while let Ok(update) = receiver.recv().await {
            // We're only interested in updates for the requested transaction hash.
            if tx_hash.0 != update.tx_hash {
                continue;
            }

            let notification = SubscriptionMessage::from_json(&update.status)?;
            subscription.send(notification).await?;
        }

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TxStatusUpdate<DaTxId> {
    tx_hash: TxHash,
    status: TxStatus<DaTxId>,
}

/// A 32-byte hash [`serde`]-encoded as a hex string optionally prefixed with
/// `0x`. See [`sov_rollup_interface::rpc::utils::rpc_hex`].
#[derive(Debug, Copy, Clone, Hash, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HexHash(#[serde(with = "sov_rollup_interface::rpc::utils::rpc_hex")] pub TxHash);

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
pub enum TxStatus<DaTxId> {
    /// The transaction was successfully submitted to a sequencer and it's
    /// sitting in the mempool waiting to be included in a batch.
    Submitted,
    /// The transaction was published to the DA as part of a batch, but it may
    /// not be finalized yet.
    Published {
        /// The ID of the DA transaction that included the rollup transaction to
        /// which this [`TxStatus`] refers.
        da_transaction_id: DaTxId,
    },
    /// The transaction was published to the DA as part of a batch that is
    /// considered finalized
    Finalized {
        /// The ID of the DA transaction that included the rollup transaction to
        /// which this [`TxStatus`] refers.
        da_transaction_id: DaTxId,
    },
}

#[cfg(test)]
mod tests {
    use sov_mock_da::{MockAddress, MockDaService};
    use sov_rollup_interface::da::BlobReaderTrait;
    use sov_rollup_interface::services::batch_builder::TxWithHash;

    use super::*;

    fn sequencer_rpc(
        batch_builder: MockBatchBuilder,
        da_service: MockDaService,
    ) -> RpcModule<Sequencer<MockBatchBuilder, MockDaService>> {
        Sequencer::new(batch_builder, da_service).rpc()
    }

    /// BatchBuilder used in tests.
    pub struct MockBatchBuilder {
        /// Mempool with transactions.
        pub mempool: Vec<Vec<u8>>,
    }

    // It only takes the first byte of the tx, when submits it.
    // This allows to show effect of batch builder
    impl BatchBuilder for MockBatchBuilder {
        fn accept_tx(&mut self, tx: Vec<u8>) -> anyhow::Result<TxHash> {
            self.mempool.push(tx);
            Ok([0; 32])
        }

        fn contains(&self, _tx_hash: &TxHash) -> bool {
            unimplemented!("MockBatchBuilder::contains is not implemented")
        }

        fn get_next_blob(&mut self) -> anyhow::Result<Vec<TxWithHash>> {
            if self.mempool.is_empty() {
                anyhow::bail!("Mock mempool is empty");
            }
            let txs = std::mem::take(&mut self.mempool)
                .into_iter()
                .filter_map(|tx| {
                    let first_byte = *tx.first()?;
                    Some(TxWithHash {
                        raw_tx: vec![first_byte],
                        hash: [0; 32],
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
        let rpc = sequencer_rpc(batch_builder, da_service);

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
        let rpc = sequencer_rpc(batch_builder, da_service.clone());

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

        let rpc = sequencer_rpc(batch_builder, da_service.clone());

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
