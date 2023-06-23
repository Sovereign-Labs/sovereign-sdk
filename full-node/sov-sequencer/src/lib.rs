use jsonrpsee::RpcModule;
// use serde::de::DeserializeOwned;
// use serde::Serialize;
use anyhow::anyhow;
use sov_rollup_interface::services::batch_builder::BatchBuilder;
use sov_rollup_interface::services::da::DaService;
use std::sync::{Arc, Mutex};

/// Single data structure that manages mempool and batch producing.
pub struct Sequencer<B: BatchBuilder, T: DaService> {
    batch_builder: Mutex<B>,
    da_service: Arc<T>,
}

impl<B: BatchBuilder + Send + Sync, T: DaService + Send + Sync> Sequencer<B, T> {
    pub fn new(batch_builder: B, da_service: Arc<T>) -> Self {
        Self {
            batch_builder: Mutex::new(batch_builder),
            da_service,
        }
    }

    async fn submit_batch(&self) -> Result<(), anyhow::Error> {
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
}

fn register_txs_rpc_methods<B, D>(
    rpc: &mut RpcModule<Sequencer<B, D>>,
) -> Result<(), jsonrpsee::core::Error>
where
    // R: Serialize + DeserializeOwned,
    // T: Serialize + DeserializeOwned,
    B: BatchBuilder + Send + Sync + 'static,
    D: DaService + Send + Sync + 'static,
{
    rpc.register_async_method("batchBuilder_submit", |_, batch_builder| async move {
        batch_builder
            .submit_batch()
            .await
            .map_err(|e| jsonrpsee::core::Error::Custom(e.to_string()))
    })?;
    Ok(())
}

pub fn get_txs_rpc<B, D>(batch_builder: B, da_service: Arc<D>) -> RpcModule<Sequencer<B, D>>
where
    // R: Serialize + DeserializeOwned,
    // T: Serialize + DeserializeOwned,
    B: BatchBuilder + Send + Sync + 'static,
    D: DaService + Send + Sync + 'static,
{
    let txs_handler = Sequencer::new(batch_builder, da_service);
    let mut rpc = RpcModule::new(txs_handler);
    register_txs_rpc_methods::<B, D>(&mut rpc).expect("Failed to register sequencer RPC methods");
    rpc
}
