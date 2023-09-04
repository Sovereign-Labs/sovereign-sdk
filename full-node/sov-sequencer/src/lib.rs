#![deny(missing_docs)]
#![doc = include_str!("../README.md")]
use std::sync::Mutex;

/// Utilities for the sequencer rpc
pub mod utils;
use anyhow::anyhow;
use jsonrpsee::types::ErrorObjectOwned;
use jsonrpsee::RpcModule;
use sov_rollup_interface::services::batch_builder::BatchBuilder;
use sov_rollup_interface::services::da::DaService;
use utils::to_jsonrpsee_error_object;

const SEQUENCER_RPC_ERROR: &str = "SEQUENCER_RPC_ERROR";

/// Single data structure that manages mempool and batch producing.
pub struct Sequencer<B: BatchBuilder, T: DaService> {
    batch_builder: Mutex<B>,
    da_service: T,
}

impl<B: BatchBuilder + Send + Sync, T: DaService + Send + Sync> Sequencer<B, T> {
    /// Creates new Sequencer from BatchBuilder and DaService
    pub fn new(batch_builder: B, da_service: T) -> Self {
        Self {
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
            let mut batch_builder = self
                .batch_builder
                .lock()
                .map_err(|e| anyhow!("failed to lock mempool: {}", e.to_string()))?;
            batch_builder.get_next_blob()?
        };
        let num_txs = blob.len();
        let blob: Vec<u8> = borsh::to_vec(&blob)?;

        match self.da_service.send_transaction(&blob).await {
            Ok(_) => Ok(num_txs),
            Err(e) => Err(anyhow!("failed to submit batch: {:?}", e)),
        }
    }

    fn accept_tx(&self, tx: Vec<u8>) -> anyhow::Result<()> {
        tracing::info!("Accepting tx: 0x{}", hex::encode(&tx));
        let mut batch_builder = self
            .batch_builder
            .lock()
            .map_err(|e| anyhow!("failed to lock mempool: {}", e.to_string()))?;
        batch_builder.accept_tx(tx)?;
        Ok(())
    }
}

fn register_txs_rpc_methods<B, D>(
    rpc: &mut RpcModule<Sequencer<B, D>>,
) -> Result<(), jsonrpsee::core::Error>
where
    B: BatchBuilder + Send + Sync + 'static,
    D: DaService,
{
    rpc.register_async_method(
        "sequencer_publishBatch",
        |params, batch_builder| async move {
            let mut params_iter = params.sequence();
            while let Some(tx) = params_iter.optional_next::<Vec<u8>>()? {
                batch_builder
                    .accept_tx(tx)
                    .map_err(|e| to_jsonrpsee_error_object(e, SEQUENCER_RPC_ERROR))?;
            }
            let num_txs = batch_builder
                .submit_batch()
                .await
                .map_err(|e| to_jsonrpsee_error_object(e, SEQUENCER_RPC_ERROR))?;

            Ok::<String, ErrorObjectOwned>(format!("Submitted {} transactions", num_txs))
        },
    )?;
    rpc.register_method("sequencer_acceptTx", move |params, sequencer| {
        let tx: SubmitTransaction = params.one()?;
        let response = match sequencer.accept_tx(tx.body) {
            Ok(()) => SubmitTransactionResponse::Registered,
            Err(e) => SubmitTransactionResponse::Failed(e.to_string()),
        };
        Ok::<_, ErrorObjectOwned>(response)
    })?;

    Ok(())
}

/// Creates an RPC module with the sequencer's methods
pub fn get_sequencer_rpc<B, D>(batch_builder: B, da_service: D) -> RpcModule<Sequencer<B, D>>
where
    B: BatchBuilder + Send + Sync + 'static,
    D: DaService,
{
    let sequencer = Sequencer::new(batch_builder, da_service);
    let mut rpc = RpcModule::new(sequencer);
    register_txs_rpc_methods::<B, D>(&mut rpc).expect("Failed to register sequencer RPC methods");
    rpc
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
    /// Submission faileds
    Failed(String),
}

#[cfg(test)]
mod tests {

    use sov_rollup_interface::da::BlobReaderTrait;
    use sov_rollup_interface::mocks::{MockAddress, MockDaService};

    use super::*;

    /// BatchBuilder used in tests.
    pub struct MockBatchBuilder {
        /// Mempool with transactions.
        pub mempool: Vec<Vec<u8>>,
    }

    // It only takes the first byte of the tx, when submits it.
    // This allows to show effect of batch builder
    impl BatchBuilder for MockBatchBuilder {
        fn accept_tx(&mut self, tx: Vec<u8>) -> anyhow::Result<()> {
            self.mempool.push(tx);
            Ok(())
        }

        fn get_next_blob(&mut self) -> anyhow::Result<Vec<Vec<u8>>> {
            if self.mempool.is_empty() {
                anyhow::bail!("Mock mempool is empty");
            }
            let txs = std::mem::take(&mut self.mempool)
                .into_iter()
                .filter_map(|tx| {
                    if !tx.is_empty() {
                        Some(vec![tx[0]])
                    } else {
                        None
                    }
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

        let mut submitted_block = da_service.get_block_at(0).await.unwrap();
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

        let mut submitted_block = da_service.get_block_at(0).await.unwrap();
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
