mod get_logs;
mod subscribe;
#[cfg(feature = "local")]
use alloy_eips::BlockId;
#[cfg(feature = "local")]
use alloy_eips::Encodable2718;
#[cfg(feature = "local")]
use alloy_primitives::Address;
#[cfg(feature = "local")]
use alloy_primitives::TxKind;
use alloy_primitives::{Bytes, B256};
use alloy_rpc_types::ReceiptEnvelope;
use alloy_rpc_types::TransactionReceipt;
#[cfg(feature = "local")]
use alloy_rpc_types::TransactionRequest;
pub use get_logs::{Cursor, LogHandlers};
use jsonrpsee::core::RpcResult;
use jsonrpsee::types::ErrorObjectOwned;
use jsonrpsee::types::Params as JRpcParams;
use jsonrpsee::Extensions;
use sov_address::{EthereumAddress, FromVmAddress};
pub use sov_evm::EthereumAuthenticator;
#[cfg(feature = "local")]
use sov_evm::Evm;
use sov_evm::RlpEvmTransaction;
use sov_metrics::RpcMetrics;
use sov_modules_api::capabilities::TransactionAuthenticator;
use sov_modules_api::capabilities::{
    AuthenticationError, AuthenticationFailureCode, AuthenticationFailureDetails, FatalError,
    HasKernel,
};
#[cfg(feature = "local")]
use sov_modules_api::macros::config_value;
use sov_modules_api::FullyBakedTx;
use sov_modules_api::Runtime;
use sov_modules_api::{RawTx, Spec};
use sov_rest_utils::{ErrorObject as RestErrorObject, GetIPResult};
#[cfg(feature = "local")]
use sov_rpc_eth_types::EthApiError;
use sov_rpc_eth_types::{LogWithExecutionTimestamp, RpcInvalidTransactionError};
use sov_sequencer::Sequencer;
use std::marker::PhantomData;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;
pub use subscribe::eth_subscribe;
use tokio::time::timeout;

use crate::Ethereum;
use crate::{rpc_internal_error, rpc_invalid_params, rpc_tx_rejected};

const TIMEOUT_CODE: i32 = 4;

type Receipt = TransactionReceipt<ReceiptEnvelope<LogWithExecutionTimestamp>>;

const MAX_TIMEOUT: u64 = 2_000; // 2 seconds

const IP_ADDRESS_ERROR: &str = "Unable to retrieve the peer IP address";

pub struct Handlers<S, Seq>(PhantomData<(S, Seq)>);

impl<S, Seq> Handlers<S, Seq>
where
    S: Spec,
    Seq: Sequencer<Spec = S>,
    S::Address: FromVmAddress<EthereumAddress>,
    Seq::Rt: HasKernel<S> + EthereumAuthenticator<S> + Default + Send + Sync + 'static,
{
    pub async fn eth_send_raw_transaction(
        parameters: JRpcParams<'static>,
        ethereum: Arc<Ethereum<S, Seq>>,
        extensions: Extensions,
    ) -> RpcResult<B256> {
        let start = Instant::now();
        let ip_addr = get_peer_ip_addr(extensions)?;

        let noop = |tx_hash, _| Ok(tx_hash);
        let result =
            Self::process_raw_transaction(parameters.one()?, ethereum, noop, ip_addr).await;

        track_metrics("eth_sendRawTransaction", start, &result);
        result
    }

    pub async fn eth_send_raw_transaction_sync(
        parameters: JRpcParams<'static>,
        ethereum: Arc<Ethereum<S, Seq>>,
        extensions: Extensions,
    ) -> RpcResult<Option<Receipt>> {
        let start = Instant::now();
        let mut params = parameters.sequence();
        let data: Bytes = params.next()?;
        let timeout_ms = params.optional_next::<u64>()?.unwrap_or(MAX_TIMEOUT);
        if timeout_ms > MAX_TIMEOUT {
            return Err(rpc_invalid_params(format!(
                "Max allowed timeout is: {MAX_TIMEOUT}"
            )));
        }
        let addr = get_peer_ip_addr(extensions)?;

        let result = timeout(
            Duration::from_millis(timeout_ms),
            Self::process_raw_transaction(data, ethereum, Self::get_receipt, addr),
        )
        .await
        .map_err(|_| {
            let err = ErrorObjectOwned::owned(
                TIMEOUT_CODE,
                format!("The transaction was added to the mempool but wasn't processed in {timeout_ms}ms."),
                None::<()>,
            );
            track_metrics("eth_sendRawTransactionSync", start, &RpcResult::<Option<Receipt>>::Err(err.clone()));
            err
        })?;
        track_metrics("eth_sendRawTransactionSync", start, &result);
        result
    }

    pub async fn realtime_send_raw_transaction(
        parameters: JRpcParams<'static>,
        ethereum: Arc<Ethereum<S, Seq>>,
        extensions: Extensions,
    ) -> RpcResult<Option<Receipt>> {
        let start = Instant::now();
        let addr = get_peer_ip_addr(extensions)?;

        let result =
            Self::process_raw_transaction(parameters.one()?, ethereum, Self::get_receipt, addr)
                .await;
        track_metrics("realtime_sendRawTransaction", start, &result);
        result
    }

    fn get_receipt(tx_hash: B256, ethereum: Arc<Ethereum<S, Seq>>) -> RpcResult<Option<Receipt>> {
        let evm = sov_evm::Evm::<S>::default();
        let state = &mut ethereum.sequencer.api_state().default_api_state_accessor();
        evm.get_transaction_receipt(tx_hash, state)
    }

    async fn process_raw_transaction<T, F>(
        data: Bytes,
        ethereum: Arc<Ethereum<S, Seq>>,
        on_success: F,
        ip_addr: IpAddr,
    ) -> RpcResult<T>
    where
        F: Fn(B256, Arc<Ethereum<S, Seq>>) -> RpcResult<T>,
    {
        let raw_evm_tx = RlpEvmTransaction { rlp: data.to_vec() };
        let (tx_hash, raw_message) = ethereum.make_raw_tx(raw_evm_tx)?;
        let tx = Seq::Rt::encode_with_ethereum_auth(RawTx::new(raw_message));
        Self::authenticate_tx(&tx, &ethereum)?;

        let seq = ethereum.sequencer.clone();
        seq.accept_tx(tx, ip_addr)
            .await
            .map_err(map_accept_tx_error)?;

        on_success(tx_hash, ethereum)
    }

    // Authenticate the transaction.
    // This was used earlier to get the credential and nonce, for retries. This has now been
    // implemented in the sequencer and is therefore no longer needed. However, calling
    // `authenticate()` here pre-calculates and caches the signature check in the async API
    // handler, which is important for performance.
    // This will also be moved into the sequencer, but for now is kept here.
    fn authenticate_tx(tx: &FullyBakedTx, ethereum: &Arc<Ethereum<S, Seq>>) -> RpcResult<()> {
        let mut state = ethereum.api_state_accessor().to_provable_reader();
        let _ = <Seq::Rt as Runtime<S>>::Auth::authenticate(tx, &mut state).map_err(|e| {
            if matches!(
                &e,
                AuthenticationError::FatalError(FatalError::InsufficientMaxFeePerGas { .. }, _)
            ) {
                return RpcInvalidTransactionError::FeeCapTooLow.into();
            }
            if let AuthenticationError::FatalError(FatalError::DeserializationFailed(err_msg), _) =
                &e
            {
                if err_msg.contains("Only EIP1559") {
                    return rpc_tx_rejected(format!("transaction type not supported: {err_msg}"));
                }
            };
            rpc_invalid_params(format!("Authentication failed: {e}"))
        })?;
        Ok(())
    }

    #[cfg(feature = "local")]
    pub async fn eth_accounts(
        _: JRpcParams<'static>,
        ethereum: Arc<Ethereum<S, Seq>>,
        _: Extensions,
    ) -> RpcResult<Vec<Address>> {
        Ok(ethereum.eth_signer.addresses())
    }

    #[cfg(feature = "local")]
    pub async fn eth_send_transaction(
        parameters: JRpcParams<'static>,
        ethereum: Arc<Ethereum<S, Seq>>,
        extensions: Extensions,
    ) -> RpcResult<B256> {
        let mut transaction_request: TransactionRequest = parameters.one()?;
        let ip_addr = get_peer_ip_addr(extensions)?;

        let evm = Evm::<S>::default();

        // get from, return error if none
        let from = transaction_request
            .from
            .ok_or_else(|| rpc_invalid_params("No from address"))?;

        // return error if not in signers
        if !ethereum.eth_signer.addresses().contains(&from) {
            return Err(rpc_invalid_params("From address not in signers"));
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

            if transaction_request.gas.is_none() {
                let estimated_gas = evm.eth_estimate_gas(
                    transaction_request.clone(),
                    Some(BlockId::pending()),
                    None,
                    None,
                    &mut state,
                )?;
                transaction_request.gas = Some(estimated_gas.to::<u64>());
            }

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
                .map_err(rpc_internal_error)?;

            RlpEvmTransaction {
                rlp: signed_tx.encoded_2718(),
            }
        };
        let (tx_hash, raw_message) = ethereum.make_raw_tx(raw_evm_tx)?;

        let tx = Seq::Rt::encode_with_ethereum_auth(RawTx::new(raw_message));

        ethereum
            .sequencer
            .accept_tx(tx, ip_addr)
            .await
            .map_err(map_accept_tx_error)?;

        Ok(tx_hash)
    }
}

fn map_accept_tx_error(err: RestErrorObject) -> ErrorObjectOwned {
    if matches!(
        serde_json::from_value::<AuthenticationFailureDetails>(serde_json::Value::Object(
            err.details.clone()
        ))
        .ok()
        .and_then(|details| details.code),
        Some(AuthenticationFailureCode::InsufficientMaxFeePerGas)
    ) {
        return RpcInvalidTransactionError::FeeCapTooLow.into();
    }

    let err_msg = format!("{} - '{}' ({:?})", err.status, err.message, err.details);
    match err.status.as_u16() {
        400 | 403 | 413 => rpc_invalid_params(err_msg),
        _ => rpc_tx_rejected(err_msg),
    }
}

fn track_metrics<T>(request_name: &'static str, start: Instant, result: &RpcResult<T>) {
    let duration = start.elapsed();
    let status = if let Err(e) = &result { e.code() } else { 0 };
    let metrics = RpcMetrics {
        request_name,
        handler_processing_time: duration,
        status,
    };

    sov_metrics::track_metrics(|tracker| {
        tracker.submit_known_metric(metrics);
    });
}

// Gets the IP needed for rete-limiting.
fn get_peer_ip_addr(extensions: Extensions) -> Result<IpAddr, ErrorObjectOwned> {
    // The `SocketAddr`` was injected into the request extensions by specific middleware in `axum::serve`.
    let ip_result = extensions.get::<GetIPResult>().ok_or_else(|| {
        tracing::error!("Axum Extensions map does not contain GetIPResult");
        rpc_internal_error(IP_ADDRESS_ERROR)
    })?;

    match ip_result.maybe_ip.as_ref() {
        Ok(ok) => Ok(*ok),
        Err(err) => {
            let err_msg = format!("IP address error: {err:?}");
            Err(rpc_internal_error(err_msg))
        }
    }
}

#[cfg(test)]
mod tests {
    use jsonrpsee::types::error::INVALID_PARAMS_CODE;

    use sov_modules_api::capabilities::{AuthenticationFailureCode, AuthenticationFailureDetails};
    use sov_rest_utils::to_json_object;

    use super::{map_accept_tx_error, RestErrorObject};

    fn sample_accept_tx_error(status: u16) -> RestErrorObject {
        let status = status.try_into().expect("status code should be valid");
        RestErrorObject {
            status,
            message: "The transaction is invalid".to_string(),
            details: Default::default(),
        }
    }

    #[test]
    fn accept_tx_bad_request_maps_to_invalid_params() {
        let err = map_accept_tx_error(sample_accept_tx_error(400));
        assert_eq!(err.code(), INVALID_PARAMS_CODE);
    }

    #[test]
    fn accept_tx_forbidden_maps_to_invalid_params() {
        let err = map_accept_tx_error(sample_accept_tx_error(403));
        assert_eq!(err.code(), INVALID_PARAMS_CODE);
    }

    #[test]
    fn accept_tx_payload_too_large_maps_to_invalid_params() {
        let err = map_accept_tx_error(sample_accept_tx_error(413));
        assert_eq!(err.code(), INVALID_PARAMS_CODE);
    }

    #[test]
    fn accept_tx_service_unavailable_stays_tx_rejected() {
        let err = map_accept_tx_error(sample_accept_tx_error(503));
        assert_eq!(err.code(), -32003);
    }

    #[test]
    fn accept_tx_low_max_fee_maps_to_invalid_input() {
        let status = 400.try_into().expect("status code should be valid");
        let err = map_accept_tx_error(RestErrorObject {
            status,
            message: "The transaction is invalid".to_string(),
            details: to_json_object(AuthenticationFailureDetails {
                error: "Insufficient max_fee_per_gas: user specified 6, but current base fee is 7"
                    .to_string(),
                code: Some(AuthenticationFailureCode::InsufficientMaxFeePerGas),
                user_max_fee_per_gas: Some(6),
                rollup_base_fee: Some(7),
            }),
        });

        assert_eq!(err.code(), -32000);
        assert_eq!(err.message(), "max fee per gas less than block base fee");
    }
}
