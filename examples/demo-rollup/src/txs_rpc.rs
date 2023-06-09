use jsonrpsee::RpcModule;
// use serde::de::DeserializeOwned;
// use serde::Serialize;
use sov_rollup_interface::services::batch_builder::BatchBuilder;
use sov_rollup_interface::services::da::DaService;
use std::sync::{Arc, RwLock};

/// Single data structure that manages mempool and batch producing.
pub struct TxsRpcHandler<B: BatchBuilder, T: DaService> {
    batch_builder: RwLock<B>,
    da_service: Arc<T>,
}

impl<B: BatchBuilder + Send + Sync, T: DaService + Send + Sync> TxsRpcHandler<B, T> {
    pub fn new(batch_builder: B, da_service: Arc<T>) -> Self {
        Self {
            batch_builder: RwLock::new(batch_builder),
            da_service,
        }
    }

    async fn submit_batch(&self) -> Result<(), anyhow::Error> {
        // Need to release lock before await, so Future is `Send`.
        let blob = {
            let mut batch_builder = self.batch_builder.write().unwrap();
            batch_builder.get_next_blob()?
        };
        let blob: Vec<u8> = blob.into_iter().flatten().collect();
        self.da_service.send_transaction(&blob).await.unwrap();
        Ok(())
    }
}

fn register_txs_rpc_methods<B, D>(
    rpc: &mut RpcModule<TxsRpcHandler<B, D>>,
) -> Result<(), jsonrpsee::core::Error>
where
    // R: Serialize + DeserializeOwned,
    // T: Serialize + DeserializeOwned,
    B: BatchBuilder + Send + Sync + 'static,
    D: DaService + Send + Sync + 'static,
{
    rpc.register_async_method("submit_batch", |_, batch_builder| async move {
        batch_builder
            .submit_batch()
            .await
            .map_err(|e| jsonrpsee::core::Error::Custom(e.to_string()))
    })?;
    Ok(())
}

pub fn get_txs_rpc<B, D>(batch_builder: B, da_service: Arc<D>) -> RpcModule<TxsRpcHandler<B, D>>
where
    // R: Serialize + DeserializeOwned,
    // T: Serialize + DeserializeOwned,
    B: BatchBuilder + Send + Sync + 'static,
    D: DaService + Send + Sync + 'static,
{
    let txs_handler = TxsRpcHandler::new(batch_builder, da_service);
    let mut rpc = RpcModule::new(txs_handler);
    register_txs_rpc_methods::<B, D>(&mut rpc).expect("Failed to register txs RPC methods");
    rpc
}
