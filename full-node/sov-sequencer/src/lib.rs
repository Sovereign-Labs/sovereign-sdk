use std::sync::{Arc, Mutex};

use anyhow::anyhow;
use jsonrpsee::RpcModule;
use sov_rollup_interface::services::batch_builder::BatchBuilder;
use sov_rollup_interface::services::da::DaService;

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
        let blob = {
            let mut batch_builder = self
                .batch_builder
                .lock()
                .map_err(|e| anyhow!("failed to lock mempool: {}", e.to_string()))?;
            batch_builder.get_next_blob()?
        };
        let blob: Vec<u8> = blob.into_iter().flatten().collect();
        match self.da_service.send_transaction(&blob).await {
            Ok(_) => Ok(()),
            Err(e) => Err(anyhow!("failed to submit batch: {:?}", e)),
        }
    }

    fn submit_transaction(&self, tx: Vec<u8>) -> anyhow::Result<()> {
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
            .map_err(|e| jsonrpsee::core::Error::Custom(e.to_string()))
    })?;
    rpc.register_method("sequencer_acceptTx", move |params, sequencer| {
        let tx: SubmitTransaction = params.one()?;
        let response = match sequencer.submit_transaction(tx.body) {
            Ok(()) => SubmitTransactionResponse::Registered,
            Err(e) => SubmitTransactionResponse::Failed(e.to_string()),
        };
        Ok(response)
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

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum SubmitTransactionResponse {
    Registered,
    Failed(String),
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Arc;

    use anyhow::bail;
    use sov_rollup_interface::da::DaSpec;
    use sov_rollup_interface::mocks::{MockDaSpec, TestBlock};

    use super::*;

    struct MockDaService {
        submitted: Arc<Mutex<Vec<Vec<u8>>>>,
    }

    impl MockDaService {
        fn new() -> Self {
            MockDaService {
                submitted: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn is_empty(&self) -> bool {
            self.submitted.lock().unwrap().is_empty()
        }

        fn get_submitted(&self) -> Vec<Vec<u8>> {
            self.submitted.lock().unwrap().clone()
        }
    }

    impl DaService for MockDaService {
        type RuntimeConfig = ();
        type Spec = MockDaSpec;
        type FilteredBlock = TestBlock;
        type Future<T> = Pin<Box<dyn Future<Output = Result<T, Self::Error>> + Send>>;
        type Error = anyhow::Error;

        fn new(
            _config: Self::RuntimeConfig,
            _chain_params: <Self::Spec as DaSpec>::ChainParams,
        ) -> Self {
            MockDaService::new()
        }

        fn get_finalized_at(&self, _height: u64) -> Self::Future<Self::FilteredBlock> {
            todo!()
        }

        fn get_block_at(&self, _height: u64) -> Self::Future<Self::FilteredBlock> {
            todo!()
        }

        fn extract_relevant_txs(
            &self,
            _block: &Self::FilteredBlock,
        ) -> Vec<<Self::Spec as DaSpec>::BlobTransaction> {
            todo!()
        }

        fn get_extraction_proof(
            &self,
            _block: &Self::FilteredBlock,
            _blobs: &[<Self::Spec as DaSpec>::BlobTransaction],
        ) -> (
            <Self::Spec as DaSpec>::InclusionMultiProof,
            <Self::Spec as DaSpec>::CompletenessProof,
        ) {
            todo!()
        }

        fn send_transaction(&self, blob: &[u8]) -> Self::Future<()> {
            self.submitted.lock().unwrap().push(blob.to_vec());
            Box::pin(async move { Ok(()) })
        }
    }

    struct MockBatchBuilder {
        mempool: Vec<Vec<u8>>,
    }

    /// It only takes the first byte of the tx, when submits it.
    /// This allows to show effect of batch builder
    impl BatchBuilder for MockBatchBuilder {
        fn accept_tx(&mut self, tx: Vec<u8>) -> anyhow::Result<()> {
            self.mempool.push(tx);
            Ok(())
        }

        fn get_next_blob(&mut self) -> anyhow::Result<Vec<Vec<u8>>> {
            if self.mempool.is_empty() {
                bail!("Mock mempool is empty");
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
        let da_service = Arc::new(MockDaService::new());
        assert!(da_service.is_empty());
        let rpc = get_sequencer_rpc(batch_builder, da_service.clone());

        let result: Result<(), jsonrpsee::core::Error> =
            rpc.call("sequencer_publishBatch", [1u64]).await;

        assert!(result.is_err());
        let error = result.err().unwrap();
        assert_eq!(
            "RPC call failed: ErrorObject { code: ServerError(-32001), message: \"Custom error: Mock mempool is empty\", data: None }",
            error.to_string());
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
        assert_eq!(vec![tx1[0], tx2[0]], submitted[0]);
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
        assert_eq!(vec![tx[0]], submitted[0]);
    }
}
