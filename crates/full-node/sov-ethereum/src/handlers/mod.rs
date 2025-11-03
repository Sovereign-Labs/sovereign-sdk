mod get_logs;
mod subscribe;
#[cfg(feature = "local")]
use alloy_primitives::TxKind;
use alloy_primitives::{Bytes, B256};
use alloy_rpc_types::TransactionReceipt;
pub use get_logs::{Cursor, LogHandlers};
use jsonrpsee::types::ErrorObjectOwned;
use jsonrpsee::types::Params as JRpcParams;
use jsonrpsee::Extensions;
use sov_address::{EthereumAddress, FromVmAddress};
pub use sov_evm::EthereumAuthenticator;
#[cfg(feature = "local")]
use sov_evm::Evm;
use sov_evm::RlpEvmTransaction;
use sov_metrics::RpcMetrics;
use sov_modules_api::capabilities::AuthorizationData;
use sov_modules_api::capabilities::HasKernel;
use sov_modules_api::capabilities::TransactionAuthenticator;
use sov_modules_api::capabilities::UniquenessData;
use sov_modules_api::Runtime;
use sov_modules_api::{RawTx, Spec};
use sov_sequencer::Sequencer;
use std::sync::Arc;
pub use subscribe::eth_subscribe;

use crate::to_jsonrpsee_error_object;
use crate::Ethereum;

const ETH_RPC_ERROR: &str = "ETH_RPC_ERROR";
/// Txs with nonce in the future of more than this threshold are rejected immediately. If the nonce is in the future but below the threshold, we'll buffer it
/// for a little while.
const FUTURE_NONCE_THRESHOLD: u64 = 100;
/// How long to wait between retries.
const SLEEP_DURATION_MS: u64 = 10;
/// The maximum amount of time to buffer a tx with a future nonce. Provides an upper bound in case retry attempts are taking too long.
const MAX_BUFFER_DURATION_MS: u128 = 200;

async fn process_raw_transaction<S, Seq, T, F>(
    data: Bytes,
    ethereum: Arc<Ethereum<S, Seq>>,
    on_success: F,
) -> Result<T, ErrorObjectOwned>
where
    S: Spec,
    Seq: Sequencer<Spec = S>,
    S::Address: FromVmAddress<EthereumAddress>,
    Seq::Rt: HasKernel<S> + EthereumAuthenticator<S> + Default + Send + Sync + 'static,
    F: Fn(B256, Arc<Ethereum<S, Seq>>) -> Result<T, ErrorObjectOwned>,
{
    let raw_evm_tx = RlpEvmTransaction { rlp: data.to_vec() };
    let (tx_hash, raw_message) = ethereum
        .make_raw_tx(raw_evm_tx)
        .map_err(|e| to_jsonrpsee_error_object(e, ETH_RPC_ERROR))?;

    // Authenticate the transaction.
    // This was used earlier to get the credential and nonce, for retries. This has now been
    // implemented in the sequencer and is therefore no longer needed. However, calling
    // `authenticate()` here pre-calculates and caches the signature check in the async API
    // handler, which is important for performance.
    // This will also be moved into the sequencer, but for now is kept here.
    let tx = Seq::Rt::encode_with_ethereum_auth(RawTx::new(raw_message));
    let mut state = ethereum
        .sequencer
        .api_state()
        .default_api_state_accessor()
        .to_provable_reader();
    let _ =
        <Seq::Rt as Runtime<S>>::Auth::authenticate(&tx, &mut state).map_err(|e| {
            to_jsonrpsee_error_object(format!("Authentication failed: {e}"), ETH_RPC_ERROR)
        })?;

    ethereum.sequencer.accept_tx(tx).await.map_err(|e| {
        to_jsonrpsee_error_object(
            format!("{} - '{}' ({:?})", e.status, e.message, e.details),
            ETH_RPC_ERROR,
        )
    })?;

    on_success(tx_hash, ethereum)
}

#[cfg(feature = "local")]
pub(crate) mod signer {
    use super::*;
    use alloy_eips::Encodable2718;
    use alloy_primitives::Address;
    use alloy_rpc_types::TransactionRequest;
    use sov_modules_api::macros::config_value;
    use sov_rpc_eth_types::EthApiError;

    pub async fn eth_accounts<S, Seq>(
        _: JRpcParams<'static>,
        ethereum: Arc<Ethereum<S, Seq>>,
        _: Extensions,
    ) -> Result<Vec<Address>, ErrorObjectOwned>
    where
        S: Spec,
        Seq: Sequencer<Spec = S>,
        S::Address: FromVmAddress<EthereumAddress>,
        Seq::Rt: HasKernel<S> + EthereumAuthenticator<S> + Default + Send + Sync + 'static,
    {
        Ok(ethereum.eth_signer.addresses())
    }

    pub async fn eth_send_transaction<S, Seq>(
        parameters: JRpcParams<'static>,
        ethereum: Arc<Ethereum<S, Seq>>,
        _: Extensions,
    ) -> Result<B256, ErrorObjectOwned>
    where
        S: Spec,
        Seq: Sequencer<Spec = S>,
        S::Address: FromVmAddress<EthereumAddress>,
        Seq::Rt: HasKernel<S> + EthereumAuthenticator<S> + Default + Send + Sync + 'static,
    {
        let mut transaction_request: TransactionRequest = parameters.one()?;

        let evm = Evm::<S>::default();

        // get from, return error if none
        let from = transaction_request
            .from
            .ok_or(to_jsonrpsee_error_object("No from address", ETH_RPC_ERROR))?;

        // return error if not in signers
        if !ethereum.eth_signer.addresses().contains(&from) {
            return Err(to_jsonrpsee_error_object(
                "From address not in signers",
                ETH_RPC_ERROR,
            ));
        }

        let raw_evm_tx = {
            let mut state = ethereum.sequencer.api_state().default_api_state_accessor();

            // set nonce if none
            transaction_request.nonce.get_or_insert_with(|| {
                evm.get_transaction_count(from, None, &mut state)
                    .unwrap_or_default()
                    .to::<u64>()
            });

            let chain_id = evm
                .chain_id(&mut state)
                .expect("Failed to get chain id")
                .map(|id| id.to())
                .unwrap_or(config_value!("CHAIN_ID"));
            transaction_request.chain_id = Some(chain_id);

            let estimated_gas = evm.eth_estimate_gas(
                transaction_request.clone(),
                Some("pending".to_string()),
                &mut state,
            )?;
            transaction_request.gas = Some(estimated_gas.to::<u64>());

            // For contract deployments, convert `to: None` to `to: Some(TxKind::Create)`
            // The JSON-RPC spec uses `null` or omitted `to` field for contract deployments,
            // but alloy's `build_typed_tx()` requires `Some(TxKind::Create)`
            if transaction_request.to.is_none() {
                transaction_request.to = Some(TxKind::Create);
            }

            let transaction = transaction_request
                .build_typed_tx()
                .map_err(|_| EthApiError::TransactionConversionError)?;

            // sign transaction
            let signed_tx = ethereum
                .eth_signer
                .sign_transaction(transaction, &from)
                .map_err(|e| to_jsonrpsee_error_object(e, ETH_RPC_ERROR))?;

            RlpEvmTransaction {
                rlp: signed_tx.encoded_2718(),
            }
        };
        let (tx_hash, raw_message) = ethereum
            .make_raw_tx(raw_evm_tx)
            .map_err(|e| to_jsonrpsee_error_object(e, ETH_RPC_ERROR))?;

        let tx = Seq::Rt::encode_with_ethereum_auth(RawTx::new(raw_message));

        ethereum.sequencer.accept_tx(tx).await.map_err(|e| {
            to_jsonrpsee_error_object(
                format!("{} - '{}' ({:?})", e.status, e.message, e.details),
                ETH_RPC_ERROR,
            )
        })?;

        Ok(tx_hash)
    }
}

pub async fn eth_send_raw_transaction<S, Seq>(
    parameters: JRpcParams<'static>,
    ethereum: Arc<Ethereum<S, Seq>>,
    _: Extensions,
) -> Result<B256, ErrorObjectOwned>
where
    S: Spec,
    Seq: Sequencer<Spec = S>,
    S::Address: FromVmAddress<EthereumAddress>,
    Seq::Rt: HasKernel<S> + EthereumAuthenticator<S> + Default + Send + Sync + 'static,
{
    let start = std::time::Instant::now();

    let data: Bytes = parameters.one()?;

    let result = process_raw_transaction(data, ethereum, |tx_hash, _| Ok(tx_hash)).await;

    // Track metrics
    {
        let duration = start.elapsed();
        let status = if let Err(e) = &result { e.code() } else { 0 };
        let metrics = RpcMetrics {
            request_name: "eth_sendRawTransaction",
            handler_processing_time: duration,
            status,
        };

        sov_metrics::track_metrics(|tracker| {
            tracker.submit(metrics);
        });
    }

    result
}

pub async fn realtime_send_raw_transaction<S, Seq>(
    parameters: JRpcParams<'static>,
    ethereum: Arc<Ethereum<S, Seq>>,
    _: Extensions,
) -> Result<Option<TransactionReceipt>, ErrorObjectOwned>
where
    S: Spec,
    Seq: Sequencer<Spec = S>,
    S::Address: FromVmAddress<EthereumAddress>,
    Seq::Rt: HasKernel<S> + EthereumAuthenticator<S> + Default + Send + Sync + 'static,
{
    let start = std::time::Instant::now();
    let data: Bytes = parameters.one()?;

    let res = process_raw_transaction(data, ethereum, |tx_hash, ethereum| {
        let evm = sov_evm::Evm::<S>::default();
        evm.get_transaction_receipt(
            tx_hash,
            &mut ethereum.sequencer.api_state().default_api_state_accessor(),
        )
    })
    .await;

    // Track metrics
    {
        let duration = start.elapsed();
        let status = if let Err(e) = &res { e.code() } else { 0 };
        let metrics = RpcMetrics {
            request_name: "realtime_sendRawTransaction",
            handler_processing_time: duration,
            status,
        };
        sov_metrics::track_metrics(|tracker| {
            tracker.submit(metrics);
        });
    }

    res
}
