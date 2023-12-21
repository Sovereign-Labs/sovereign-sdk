use std::{fmt::Display, hash::Hash, str::FromStr};

use jsonrpsee::{types::ErrorObjectOwned, RpcModule};
use sov_modules_api::utils::to_jsonrpsee_error_object;
use sov_rollup_interface::services::{batch_builder::BatchBuilder, da::DaService};

use crate::{Sequencer, SubmitTransaction, SubmitTransactionResponse, TxStatus};

const SEQUENCER_RPC_ERROR: &str = "SEQUENCER_RPC_ERROR";

/// Creates an RPC module with the sequencer's methods
pub fn get_sequencer_rpc<B, D>(batch_builder: B, da_service: D) -> RpcModule<Sequencer<B, D>>
where
    B: BatchBuilder + Send + Sync + 'static,
    B::TxHash: Hash + Eq + Clone + FromStr + ToString + Send + Sync,
    <B::TxHash as FromStr>::Err: Display,
    D: DaService,
{
    let sequencer = Sequencer::new(batch_builder, da_service);
    let mut rpc = RpcModule::new(sequencer);
    register_txs_rpc_methods::<B, D>(&mut rpc).expect("Failed to register sequencer RPC methods");
    rpc
}

fn register_txs_rpc_methods<B, D>(
    rpc: &mut RpcModule<Sequencer<B, D>>,
) -> Result<(), jsonrpsee::core::Error>
where
    B: BatchBuilder + Send + Sync + 'static,
    B::TxHash: Hash + Eq + Clone + FromStr + ToString + Send + Sync,
    <B::TxHash as FromStr>::Err: Display,
    D: DaService,
{
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
        let tx_hash_str: String = params.one()?;
        let tx_hash = B::TxHash::from_str(&tx_hash_str).map_err(|err| {
            to_jsonrpsee_error_object(
                format!("invalid tx hash value: {}", err),
                SEQUENCER_RPC_ERROR,
            )
        })?;

        let is_in_mempool = sequencer.batch_builder.lock().await.contains(&tx_hash);
        let status = if is_in_mempool {
            Some(TxStatus::Submitted)
        } else {
            sequencer.tx_statuses_cache.get(&tx_hash)
        };

        Ok::<_, ErrorObjectOwned>(status)
    })?;
    rpc.register_subscription(
        "sequencer_subscribeToTxStatusUpdates",
        "sequencer_newTxStatus",
        "sequencer_unsubscribeToTxStatusUpdates",
        |params, pending, sequencer| async move {
            sequencer
                .handle_tx_status_update_subscription(params, pending)
                .await
        },
    )?;

    Ok(())
}
