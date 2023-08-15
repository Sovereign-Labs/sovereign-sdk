use std::sync::{Arc, Mutex};

use anyhow::anyhow;
use jsonrpsee::types::ErrorObjectOwned;
use jsonrpsee::RpcModule;
use sov_modules_api::utils::to_jsonrpsee_error_object;
use sov_rollup_interface::services::batch_builder::BatchBuilder;
use sov_rollup_interface::services::da::DaService;

const SEQUENCER_RPC_ERROR: &str = "SEQUENCER_RPC_ERROR";

/// Single data structure that manages mempool and batch producing.
pub struct Sequencer<B: BatchBuilder, T: DaService> {
    batch_builder: Mutex<B>,
    da_service: Arc<T>,
}

impl<B: BatchBuilder + Send + Sync, T: DaService + Send + Sync> Sequencer<B, T> {
    /// Creates new Sequencer from BatchBuilder and DaService
    pub fn new(batch_builder: B, da_service: Arc<T>) -> Self {
        Self {
            batch_builder: Mutex::new(batch_builder),
            da_service,
        }
    }

    async fn submit_batch(&self) -> anyhow::Result<()> {
        // Need to release lock before await, so Future is `Send`.
        // But potentially it can create blobs that sent out of order.
        // Can be improved with atomics, so new batch is only created after previous was submitted.
        tracing::info!("Going to submit batch!");
        let blob = {
            let mut batch_builder = self
                .batch_builder
                .lock()
                .map_err(|e| anyhow!("failed to lock mempool: {}", e.to_string()))?;
            batch_builder.get_next_blob()?
        };
        let blob: Vec<u8> = borsh::to_vec(&blob)?;
        match self.da_service.send_transaction(&blob).await {
            Ok(_) => Ok(()),
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
    D: DaService + Send + Sync + 'static,
{
    rpc.register_async_method("sequencer_publishBatch", |_, batch_builder| async move {
        batch_builder
            .submit_batch()
            .await
            .map_err(|e| to_jsonrpsee_error_object(e, SEQUENCER_RPC_ERROR))
    })?;
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

pub fn get_sequencer_rpc<B, D>(batch_builder: B, da_service: Arc<D>) -> RpcModule<Sequencer<B, D>>
where
    B: BatchBuilder + Send + Sync + 'static,
    D: DaService + Send + Sync + 'static,
{
    let sequencer = Sequencer::new(batch_builder, da_service);
    let mut rpc = RpcModule::new(sequencer);
    register_txs_rpc_methods::<B, D>(&mut rpc).expect("Failed to register sequencer RPC methods");
    rpc
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct SubmitTransaction {
    body: Vec<u8>,
}

impl SubmitTransaction {
    pub fn new(body: Vec<u8>) -> Self {
        SubmitTransaction { body }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum SubmitTransactionResponse {
    Registered,
    Failed(String),
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use sov_rollup_interface::mocks::{MockBatchBuilder, MockDaService};

    use super::*;

    #[tokio::test]
    async fn test_submit_on_empty_mempool() {
        let batch_builder = MockBatchBuilder { mempool: vec![] };
        let da_service = Arc::new(MockDaService::new());
        assert!(da_service.is_empty());
        let rpc = get_sequencer_rpc(batch_builder, da_service.clone());

        let result: Result<(), jsonrpsee::core::Error> =
            rpc.call("sequencer_publishBatch", [1u64]).await;

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
        let da_service = Arc::new(MockDaService::new());
        assert!(da_service.is_empty());
        let rpc = get_sequencer_rpc(batch_builder, da_service.clone());

        let _: () = rpc.call("sequencer_publishBatch", [1u64]).await.unwrap();

        assert!(!da_service.is_empty());

        let submitted = da_service.get_submitted();
        assert_eq!(1, submitted.len());
        // First bytes of each tx, flattened
        let blob: Vec<Vec<u8>> = vec![vec![tx1[0]], vec![tx2[0]]];
        let expected: Vec<u8> = borsh::to_vec(&blob).unwrap();
        assert_eq!(expected, submitted[0]);
    }

    #[tokio::test]
    async fn test_accept_tx() {
        let batch_builder = MockBatchBuilder { mempool: vec![] };
        let da_service = Arc::new(MockDaService::new());

        let rpc = get_sequencer_rpc(batch_builder, da_service.clone());
        assert!(da_service.is_empty());

        let tx: Vec<u8> = vec![1, 2, 3, 4, 5];
        let request = SubmitTransaction { body: tx.clone() };
        let result: SubmitTransactionResponse =
            rpc.call("sequencer_acceptTx", [request]).await.unwrap();
        assert_eq!(SubmitTransactionResponse::Registered, result);

        // Check that it got passed to DA service
        assert!(da_service.is_empty());

        let _: () = rpc.call("sequencer_publishBatch", [1u64]).await.unwrap();

        assert!(!da_service.is_empty());

        let submitted = da_service.get_submitted();
        assert_eq!(1, submitted.len());
        // First bytes of each tx, flattened
        let blob: Vec<Vec<u8>> = vec![vec![tx[0]]];
        let expected: Vec<u8> = borsh::to_vec(&blob).unwrap();
        assert_eq!(expected, submitted[0]);
    }

    #[tokio::test]
    #[ignore = "TBD"]
    async fn test_full_flow() {}
}
