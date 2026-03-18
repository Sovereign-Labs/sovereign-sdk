mod get_logs;
mod subscribe;
use alloy_eips::BlockId;
#[cfg(feature = "local")]
use alloy_eips::Encodable2718;
#[cfg(feature = "local")]
use alloy_primitives::Address;
#[cfg(feature = "local")]
use alloy_primitives::TxKind;
use alloy_primitives::{Bytes, B256, U256, U64};
use alloy_rpc_types::state::StateOverride;
use alloy_rpc_types::BlockOverrides;
use alloy_rpc_types::ReceiptEnvelope;
use alloy_rpc_types::TransactionReceipt;
use alloy_rpc_types::TransactionRequest;
pub use get_logs::{Cursor, LogHandlers};
use jsonrpsee::core::RpcResult;
use jsonrpsee::types::ErrorObjectOwned;
use jsonrpsee::types::Params as JRpcParams;
use jsonrpsee::Extensions;
use serde::Deserialize;
use sov_address::{EthereumAddress, FromVmAddress};
pub use sov_evm::EthereumAuthenticator;
use sov_evm::RlpEvmTransaction;
use sov_evm::{build_request_preflight_auth, Evm, TransactionSigned};
use sov_metrics::RpcMetrics;
use sov_modules_api::capabilities::{
    AuthenticationError, AuthorizationData, FatalError, GasEnforcer, HasCapabilities, HasKernel,
    TransactionAuthenticator, TransactionAuthorizer,
};
#[cfg(feature = "local")]
use sov_modules_api::macros::config_value;
use sov_modules_api::transaction::{
    AuthenticatedTransactionAndRawHash, AuthenticatedTransactionData,
};
use sov_modules_api::ApiStateAccessor;
use sov_modules_api::ExecutionContext;
use sov_modules_api::FullyBakedTx;
use sov_modules_api::GetGasPrice;
use sov_modules_api::Runtime;
use sov_modules_api::{RawTx, Spec};
use sov_rest_utils::{ErrorObject as RestErrorObject, GetIPResult};
use sov_rpc_eth_types::{EthApiError, LogWithExecutionTimestamp, RpcInvalidTransactionError};
use sov_sequencer::{AcceptTxErrorCode, Sequencer};
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

enum AffordabilityPreflight {
    Affordable,
    Rejected(ErrorObjectOwned),
    Skip,
}

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

    pub async fn eth_estimate_gas(
        parameters: JRpcParams<'static>,
        ethereum: Arc<Ethereum<S, Seq>>,
        _: Extensions,
    ) -> RpcResult<U64> {
        let mut params = parameters.sequence();
        let request: TransactionRequest = params.next()?;
        let block_id: Option<BlockId> = params.optional_next()?;
        let state_overrides: Option<StateOverride> = params.optional_next()?;
        let block_overrides: Option<Box<BlockOverrides>> = params.optional_next()?;

        Self::estimate_gas_request(
            request,
            block_id,
            state_overrides,
            block_overrides,
            &ethereum,
        )
    }

    fn estimate_gas_request(
        request: TransactionRequest,
        block_id: Option<BlockId>,
        state_overrides: Option<StateOverride>,
        block_overrides: Option<Box<BlockOverrides>>,
        ethereum: &Arc<Ethereum<S, Seq>>,
    ) -> RpcResult<U64> {
        let evm = Evm::<S>::default();
        // Pin one checkpoint snapshot for the full request so validation,
        // affordability preflight, and estimation cannot observe different heads.
        let snapshot_state = ethereum.api_state_accessor();

        {
            let mut validation_state = snapshot_state.clone_without_local_writes();
            evm.validate_estimate_gas_request(
                &request,
                block_id,
                state_overrides.clone(),
                block_overrides.clone(),
                &mut validation_state,
            )?;
        }

        if Self::supports_request_affordability_preflight(
            &request,
            state_overrides.as_ref(),
            block_overrides.as_deref(),
        ) {
            let mut api_state = snapshot_state.clone_without_local_writes();
            let mut preflight_state = evm
                .preflight_state_for_block_id(block_id, &mut api_state)
                .map_err(ErrorObjectOwned::from)?;

            match Self::request_affordability_preflight(&request, &mut preflight_state, ethereum)? {
                AffordabilityPreflight::Affordable | AffordabilityPreflight::Skip => {}
                AffordabilityPreflight::Rejected(err) => return Err(err),
            }
        }

        let mut state = snapshot_state.clone_without_local_writes();
        evm.eth_estimate_gas(
            request,
            block_id,
            state_overrides,
            block_overrides,
            &mut state,
        )
    }

    /// Returns whether a real-state affordability preflight is meaningful for this request.
    ///
    /// When state or block overrides are present, account balances and gas pricing can be
    /// arbitrarily changed by the caller, so a preflight against real state would be
    /// meaningless. The EVM execution itself enforces affordability under the overridden state.
    fn supports_request_affordability_preflight(
        request: &TransactionRequest,
        state_overrides: Option<&StateOverride>,
        block_overrides: Option<&BlockOverrides>,
    ) -> bool {
        request.from.is_some()
            && request.gas.is_some()
            && request.max_fee_per_gas.or(request.gas_price).is_some()
            && state_overrides.is_none()
            && block_overrides.is_none()
    }

    fn request_affordability_preflight(
        request: &TransactionRequest,
        state: &mut ApiStateAccessor<S>,
        ethereum: &Arc<Ethereum<S, Seq>>,
    ) -> RpcResult<AffordabilityPreflight> {
        let Some(from) = request.from else {
            return Ok(AffordabilityPreflight::Skip);
        };
        let Some(gas_limit) = request.gas else {
            return Ok(AffordabilityPreflight::Skip);
        };
        let Some(max_fee_per_gas) = request.max_fee_per_gas.or(request.gas_price) else {
            return Ok(AffordabilityPreflight::Skip);
        };

        let mut auth_state = state.clone_without_local_writes().to_provable_reader();
        let (authenticated_tx, auth_data) =
            match build_request_preflight_auth::<_, S>(request, &mut auth_state) {
                Ok(data) => data,
                Err(err) => {
                    tracing::warn!(
                        error = %err,
                        "skipping request affordability preflight: unable to build auth data"
                    );
                    return Ok(AffordabilityPreflight::Skip);
                }
            };

        Self::run_affordability_preflight(
            &authenticated_tx,
            &auth_data,
            S::Address::from_vm_address(EthereumAddress::from(from)),
            gas_limit,
            max_fee_per_gas,
            request.value.unwrap_or_default(),
            state,
            ethereum,
        )
    }

    fn run_affordability_preflight(
        authenticated_tx: &AuthenticatedTransactionData<S>,
        auth_data: &AuthorizationData<S>,
        sender_rollup_addr: S::Address,
        requested_gas_limit: u64,
        requested_fee_per_gas: u128,
        requested_value: U256,
        state: &mut ApiStateAccessor<S>,
        ethereum: &Arc<Ethereum<S, Seq>>,
    ) -> RpcResult<AffordabilityPreflight> {
        let evm = Evm::<S>::default();
        let fee_check_active = evm
            .is_max_fee_check_active(state)
            .map_err(|e| rpc_internal_error(format!("state read error: {e}")))?;
        if !fee_check_active {
            return Ok(AffordabilityPreflight::Skip);
        }

        let requested_gas_cost = U256::from(requested_gas_limit)
            .checked_mul(U256::from(requested_fee_per_gas))
            .ok_or_else(|| {
                ErrorObjectOwned::from(EthApiError::InvalidTransaction(
                    RpcInvalidTransactionError::GasUintOverflow,
                ))
            })?;
        let requested_total_cost =
            requested_gas_cost
                .checked_add(requested_value)
                .ok_or_else(|| {
                    ErrorObjectOwned::from(EthApiError::InvalidTransaction(
                        RpcInvalidTransactionError::GasUintOverflow,
                    ))
                })?;

        let sender_balance = Self::read_balance(&sender_rollup_addr, state)?;
        if requested_total_cost.is_zero() {
            return Ok(AffordabilityPreflight::Affordable);
        }

        let mut runtime = Seq::Rt::default();
        let gas_price = state.gas_price();
        let mut context = match runtime.transaction_authorizer().resolve_context(
            auth_data,
            &ethereum.sequencer_da_address,
            ethereum.sequencer_rollup_address,
            state,
            None,
            ExecutionContext::Sequencer,
            ethereum.sequencer_type,
        ) {
            Ok(context) => context,
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    "skipping affordability preflight: unable to resolve runtime context"
                );
                return Ok(AffordabilityPreflight::Skip);
            }
        };

        if let Err(err) =
            runtime
                .gas_enforcer()
                .try_reserve_gas(authenticated_tx, gas_price, &mut context, state)
        {
            tracing::warn!(
                error = %err,
                sender = %sender_rollup_addr,
                "affordability preflight rejected while reserving gas"
            );
            return Ok(AffordabilityPreflight::Rejected(
                Self::insufficient_funds_error(requested_total_cost, sender_balance),
            ));
        }

        let actual_payer = *context.gas_refund_recipient();

        if actual_payer == sender_rollup_addr {
            // Sender pays everything: gas + value.
            if sender_balance < requested_total_cost {
                return Ok(AffordabilityPreflight::Rejected(
                    Self::insufficient_funds_error(requested_total_cost, sender_balance),
                ));
            }
            return Ok(AffordabilityPreflight::Affordable);
        }

        // Paymaster path: paymaster covers gas, sender covers value.
        let payer_balance_after = Self::read_balance(&actual_payer, state)?;
        let payer_balance_before = payer_balance_after
            .checked_add(U256::from(authenticated_tx.0.max_fee.0))
            .ok_or_else(|| {
                ErrorObjectOwned::from(EthApiError::InvalidTransaction(
                    RpcInvalidTransactionError::GasUintOverflow,
                ))
            })?;

        if payer_balance_before < requested_gas_cost {
            return Ok(AffordabilityPreflight::Rejected(
                Self::insufficient_funds_error(requested_gas_cost, payer_balance_before),
            ));
        }

        if sender_balance < requested_value {
            return Ok(AffordabilityPreflight::Rejected(ErrorObjectOwned::from(
                EthApiError::InvalidTransaction(
                    RpcInvalidTransactionError::InsufficientFundsForTransfer,
                ),
            )));
        }

        Ok(AffordabilityPreflight::Affordable)
    }

    fn read_balance(account: &S::Address, state: &mut ApiStateAccessor<S>) -> RpcResult<U256> {
        let bank = sov_bank::Bank::<S>::default();
        bank.get_balance_of(account, sov_bank::config_gas_token_id(), state)
            .map_err(|e| rpc_internal_error(format!("balance read error: {e}")))
            .map(|maybe_amount| {
                maybe_amount
                    .map(|amount| U256::from(amount.0))
                    .unwrap_or_default()
            })
    }

    fn insufficient_funds_error(cost: U256, balance: U256) -> ErrorObjectOwned {
        ErrorObjectOwned::from(EthApiError::InvalidTransaction(
            RpcInvalidTransactionError::InsufficientFunds { cost, balance },
        ))
    }

    fn decode_raw_transaction(data: &Bytes) -> RpcResult<(B256, Vec<u8>, TransactionSigned)> {
        let raw_tx = RlpEvmTransaction { rlp: data.to_vec() };
        let message = borsh::to_vec(&raw_tx).expect("Failed to serialize raw tx");
        let signed_tx = sov_evm::convert_to_tx_signed(raw_tx)
            .map_err(|err| ErrorObjectOwned::from(EthApiError::from(err)))?;
        let tx_hash = *signed_tx.hash();

        Ok((tx_hash, message, signed_tx))
    }

    fn raw_transaction_affordability_preflight(
        signed_tx: &TransactionSigned,
        authenticated_tx: &AuthenticatedTransactionAndRawHash<S>,
        auth_data: &AuthorizationData<S>,
        ethereum: &Arc<Ethereum<S, Seq>>,
    ) -> RpcResult<()> {
        use alloy_consensus::transaction::SignerRecoverable;
        use alloy_consensus::Transaction;

        let sender = signed_tx
            .recover_signer()
            .map_err(|_| rpc_invalid_params("failed to recover signer"))?;
        let sender_rollup_addr = S::Address::from_vm_address(EthereumAddress::from(sender));
        let mut state = ethereum.api_state_accessor();

        match Self::run_affordability_preflight(
            &authenticated_tx.authenticated_tx,
            auth_data,
            sender_rollup_addr,
            signed_tx.gas_limit(),
            signed_tx.max_fee_per_gas(),
            signed_tx.value(),
            &mut state,
            ethereum,
        )? {
            AffordabilityPreflight::Affordable | AffordabilityPreflight::Skip => Ok(()),
            AffordabilityPreflight::Rejected(err) => Err(err),
        }
    }

    fn authenticate_tx(
        tx: &FullyBakedTx,
        ethereum: &Arc<Ethereum<S, Seq>>,
    ) -> RpcResult<(AuthenticatedTransactionAndRawHash<S>, AuthorizationData<S>)> {
        let mut state = ethereum.api_state_accessor().to_provable_reader();
        let (authenticated_tx, auth_data, _) =
            <Seq::Rt as Runtime<S>>::Auth::authenticate(tx, &mut state)
                .map_err(map_authentication_error)?;
        Ok((authenticated_tx, auth_data))
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
        let (tx_hash, raw_message, signed_tx) = Self::decode_raw_transaction(&data)?;
        let tx = Seq::Rt::encode_with_ethereum_auth(RawTx::new(raw_message));
        {
            let (authenticated_tx, auth_data) = Self::authenticate_tx(&tx, &ethereum)?;
            Self::raw_transaction_affordability_preflight(
                &signed_tx,
                &authenticated_tx,
                &auth_data,
                &ethereum,
            )?;
        }

        let seq = ethereum.sequencer.clone();
        seq.accept_tx(tx, ip_addr)
            .await
            .map_err(map_accept_tx_error)?;

        on_success(tx_hash, ethereum)
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

        {
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
        }

        if transaction_request.gas.is_none() {
            let estimated_gas = Self::estimate_gas_request(
                transaction_request.clone(),
                Some(BlockId::pending()),
                None,
                None,
                &ethereum,
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

        let signed_tx = ethereum
            .eth_signer
            .sign_transaction(transaction, &from)
            .map_err(rpc_internal_error)?;

        // Inline the submit path instead of going through `process_raw_transaction`,
        // which would redundantly RLP-decode and re-recover the signer we just signed with.
        let tx_hash = *signed_tx.hash();
        let raw_tx = RlpEvmTransaction {
            rlp: signed_tx.encoded_2718(),
        };
        let message = borsh::to_vec(&raw_tx).expect("Failed to serialize raw tx");
        let tx = Seq::Rt::encode_with_ethereum_auth(RawTx::new(message));

        {
            let (authenticated_tx, auth_data) = Self::authenticate_tx(&tx, &ethereum)?;
            Self::raw_transaction_affordability_preflight(
                &signed_tx,
                &authenticated_tx,
                &auth_data,
                &ethereum,
            )?;
        }

        let seq = ethereum.sequencer.clone();
        seq.accept_tx(tx, ip_addr)
            .await
            .map_err(map_accept_tx_error)?;

        Ok(tx_hash)
    }
}

fn map_authentication_error(error: AuthenticationError) -> ErrorObjectOwned {
    match &error {
        AuthenticationError::FatalError(FatalError::DeserializationFailed(err_msg), _)
            if err_msg.contains("Only EIP-1559") =>
        {
            rpc_tx_rejected(format!("transaction type not supported: {err_msg}"))
        }
        AuthenticationError::FatalError(FatalError::InsufficientMaxFeePerGas { .. }, _) => {
            rpc_fee_cap_too_low()
        }
        _ => rpc_invalid_params(format!("Authentication failed: {error}")),
    }
}

fn rpc_fee_cap_too_low() -> ErrorObjectOwned {
    ErrorObjectOwned::from(EthApiError::InvalidTransaction(
        RpcInvalidTransactionError::FeeCapTooLow,
    ))
}

fn accept_tx_error_code(err: &RestErrorObject) -> Option<AcceptTxErrorCode> {
    let code_value = err.details.get("code")?;
    AcceptTxErrorCode::deserialize(code_value).ok()
}

fn map_accept_tx_error(err: RestErrorObject) -> ErrorObjectOwned {
    if matches!(
        accept_tx_error_code(&err),
        Some(AcceptTxErrorCode::InsufficientMaxFeePerGas)
    ) {
        return rpc_fee_cap_too_low();
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
    use alloy_rpc_types::error::EthRpcErrorCode;
    use jsonrpsee::types::error::INVALID_PARAMS_CODE;
    use sov_modules_api::capabilities::{AuthenticationError, FatalError};
    use sov_modules_api::TxHash;
    use sov_rest_utils::to_json_object;

    use super::{map_accept_tx_error, map_authentication_error, RestErrorObject};
    use sov_sequencer::{AcceptTxErrorCode, AcceptTxErrorDetails};

    fn sample_accept_tx_error(status: u16) -> RestErrorObject {
        let status = status.try_into().expect("status code should be valid");
        RestErrorObject {
            status,
            message: "The transaction is invalid".to_string(),
            details: Default::default(),
        }
    }

    fn sample_accept_tx_error_with_details(
        status: u16,
        details: AcceptTxErrorDetails,
    ) -> RestErrorObject {
        let status = status.try_into().expect("status code should be valid");
        RestErrorObject {
            status,
            message: "The transaction is invalid".to_string(),
            details: to_json_object(details),
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
    fn accept_tx_low_fee_cap_maps_to_invalid_input() {
        let err = map_accept_tx_error(sample_accept_tx_error_with_details(
            400,
            AcceptTxErrorDetails {
                code: Some(AcceptTxErrorCode::InsufficientMaxFeePerGas),
                error: Some("Authentication failed".to_string()),
            },
        ));

        assert_eq!(err.code(), EthRpcErrorCode::InvalidInput.code());
        assert_eq!(err.message(), "max fee per gas less than block base fee");
    }

    #[test]
    fn authentication_low_fee_cap_maps_to_invalid_input() {
        let err = map_authentication_error(AuthenticationError::FatalError(
            FatalError::InsufficientMaxFeePerGas {
                user_max_fee_per_gas: 1,
                rollup_base_fee: 2,
            },
            TxHash::new([0; 32]),
        ));

        assert_eq!(err.code(), EthRpcErrorCode::InvalidInput.code());
        assert_eq!(err.message(), "max fee per gas less than block base fee");
    }
}
