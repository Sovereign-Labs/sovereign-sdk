use crate::handlers::Handlers;
use crate::Ethereum;
use alloy_eips::BlockId;
use alloy_primitives::{U256, U64};
use alloy_rpc_types::state::StateOverride;
use alloy_rpc_types::{BlockOverrides, TransactionRequest};
use jsonrpsee::core::RpcResult;
use jsonrpsee::types::ErrorObjectOwned;
use jsonrpsee::types::Params as JRpcParams;
use jsonrpsee::Extensions;
use sov_address::{EthereumAddress, FromVmAddress};
use sov_evm::{build_request_preflight_auth, EthereumAuthenticator, Evm};
use sov_metrics::{AuthAndProcessMetrics, AuthAndProcessTimings};
use sov_modules_api::capabilities::GasEnforcer;
use sov_modules_api::capabilities::{
    AuthorizationData, HasCapabilities, HasKernel, TransactionAuthorizer,
};
use sov_modules_api::{
    ApiStateAccessor, AuthenticatedTransactionData, ExecutionContext, GasArray, GetGasPrice,
    NoOpControlFlow, Spec, TxEffect,
};
use sov_modules_stf_blueprint::process_tx_and_reward_prover;
use sov_rpc_eth_types::{EthApiError, RpcInvalidTransactionError};
use sov_sequencer::Sequencer;
use std::sync::Arc;

pub(crate) enum AffordabilityPreflight {
    Affordable,
    Rejected(ErrorObjectOwned),
    Skip,
}

impl<S, Seq> Handlers<S, Seq>
where
    S: Spec,
    Seq: Sequencer<Spec = S>,
    S::Address: FromVmAddress<EthereumAddress>,
    Seq::Rt: HasKernel<S> + EthereumAuthenticator<S> + Default + Send + Sync + 'static,
{
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
        // Pin one checkpoint snapshot for the full request so validation,
        // affordability preflight, and estimation cannot observe different heads.
        let snapshot_state = ethereum.api_state_accessor();

        Self::estimate_gas_request(
            request,
            block_id,
            state_overrides,
            block_overrides,
            &snapshot_state,
            &ethereum,
        )
    }

    pub(crate) fn estimate_gas_request(
        request: TransactionRequest,
        block_id: Option<BlockId>,
        state_overrides: Option<StateOverride>,
        block_overrides: Option<Box<BlockOverrides>>,
        snapshot_state: &ApiStateAccessor<S>,
        ethereum: &Arc<Ethereum<S, Seq>>,
    ) -> RpcResult<U64> {
        let evm = Evm::<S>::default();
        // When state or block overrides are present, account balances and gas pricing can be
        // arbitrarily changed by the caller, so a preflight against real state would be
        // meaningless.
        // The EVM execution itself enforces affordability under the overridden state.
        let has_overrides = state_overrides.is_some() || block_overrides.is_some();
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

        Self::validate_request_stale_nonce_preflight(&request, block_id, snapshot_state)?;

        let has_explicit_gas = request.gas.is_some();
        if has_explicit_gas && !has_overrides {
            Self::run_request_affordability_preflight(
                &request,
                block_id,
                snapshot_state,
                ethereum,
            )?;
        }

        // Runtime parity only covers the no-override path with a concrete sender.
        // Override requests still need the fallback estimator because it applies RPC
        // state/block overrides that the runtime-parity path does not support.
        let estimated_gas = if Self::should_use_runtime_parity_estimate(&request, has_overrides) {
            Self::estimate_gas_via_runtime_parity(
                request.clone(),
                block_id,
                snapshot_state,
                ethereum,
            )?
        } else {
            let mut state = snapshot_state.clone_without_local_writes();
            evm.eth_estimate_gas_helper(
                request.clone(),
                block_id,
                state_overrides.clone(),
                block_overrides.clone(),
                &mut state,
            )?
        };

        if !has_explicit_gas && !has_overrides {
            let mut request_with_estimated_gas = request;
            request_with_estimated_gas.gas = Some(estimated_gas.to::<u64>());
            Self::run_request_affordability_preflight(
                &request_with_estimated_gas,
                block_id,
                snapshot_state,
                ethereum,
            )?;
        }

        Ok(estimated_gas)
    }

    fn estimate_gas_via_runtime_parity(
        request: TransactionRequest,
        block_id: Option<BlockId>,
        snapshot_state: &ApiStateAccessor<S>,
        ethereum: &Arc<Ethereum<S, Seq>>,
    ) -> Result<U64, ErrorObjectOwned> {
        let evm = Evm::<S>::default();
        let prepared = evm
            .prepare_runtime_parity_estimate::<Seq::Rt>(
                request,
                block_id,
                snapshot_state,
                ethereum.sequencer_type,
            )
            .map_err(|err| ErrorObjectOwned::owned(-32603, err, None::<()>))?;
        let metrics = AuthAndProcessMetrics::new(
            prepared.authenticated_tx.raw_tx_hash.0,
            AuthAndProcessTimings::new_with_defaults(ExecutionContext::Sequencer.str()),
        );
        let validated_output = (
            prepared.authenticated_tx,
            prepared.auth_data,
            prepared.runtime_call,
        );

        let mut runtime = Seq::Rt::default();
        let (result, mut tx_scratchpad, _) = process_tx_and_reward_prover(
            &mut runtime,
            prepared.pre_exec_working_set,
            // `eth_estimateGas` should not depend on transient remaining slot budget.
            // We disable the STF slot-gas clamp here and still remain bounded by the
            // transaction's own gas limit inside the real execution pipeline.
            <S::Gas>::MAX,
            validated_output,
            prepared.raw_tx,
            &ethereum.sequencer_da_address,
            ethereum.sequencer_rollup_address,
            ExecutionContext::Sequencer,
            &NoOpControlFlow,
            prepared.operating_mode,
            metrics,
            ethereum.sequencer_type,
        );

        match result {
            Ok(apply_tx_result) => match apply_tx_result.receipt.receipt {
                TxEffect::Successful(_) => evm
                    .read_runtime_parity_estimate_from_pending_tail(&mut tx_scratchpad)
                    .map_err(|err| ErrorObjectOwned::owned(-32603, err, None::<()>)),
                other => Err(Self::tx_effect_to_rpc_error(other)),
            },
            Err((error, _)) => Err(ErrorObjectOwned::owned(
                alloy_rpc_types::error::EthRpcErrorCode::TransactionRejected.code(),
                error.to_string(),
                None::<()>,
            )),
        }
    }

    /// Converts a non-successful `TxEffect` into the appropriate RPC error.
    /// Transaction-level failures (reverts, skips) use the standard transaction-rejected
    /// error code so that RPC clients see the same classification as `eth_sendRawTransaction`.
    fn tx_effect_to_rpc_error(effect: TxEffect<S>) -> ErrorObjectOwned {
        let code = alloy_rpc_types::error::EthRpcErrorCode::TransactionRejected.code();
        match effect {
            TxEffect::Reverted(contents) => {
                ErrorObjectOwned::owned(code, contents.reason.to_string(), None::<()>)
            }
            TxEffect::Skipped(contents) => {
                ErrorObjectOwned::owned(code, contents.error.to_string(), None::<()>)
            }
            other => ErrorObjectOwned::owned(
                code,
                format!("runtime parity transaction was not successful: {other:?}"),
                None::<()>,
            ),
        }
    }

    fn should_use_runtime_parity_estimate(
        request: &TransactionRequest,
        has_overrides: bool,
    ) -> bool {
        !has_overrides && request.from.is_some()
    }

    pub(crate) fn run_request_affordability_preflight(
        request: &TransactionRequest,
        block_id: Option<BlockId>,
        snapshot_state: &ApiStateAccessor<S>,
        ethereum: &Arc<Ethereum<S, Seq>>,
    ) -> RpcResult<()> {
        let Some(from) = request.from else {
            return Ok(());
        };
        // This guard is defensive: omitted-gas callers synthesize `gas` from a pinned estimate
        // before calling this helper.
        if request.gas.is_none() {
            return Ok(());
        }
        if request.max_fee_per_gas.or(request.gas_price).is_none() {
            return Ok(());
        }

        let evm = Evm::<S>::default();
        let mut api_state = snapshot_state.clone_without_local_writes();
        let preflight_state = evm
            .preflight_state_for_block_id(block_id, &mut api_state)
            .map_err(ErrorObjectOwned::from)?;
        let mut affordability_state = preflight_state.clone_without_local_writes();

        let mut auth_state = preflight_state
            .clone_without_local_writes()
            .to_provable_reader();
        let (authenticated_tx, auth_data) =
            match build_request_preflight_auth::<_, S>(request, &mut auth_state) {
                Ok(data) => data,
                Err(err) => {
                    tracing::warn!(
                        error = %err,
                        "skipping request affordability preflight: unable to build auth data"
                    );
                    // Wrapper preflight is best-effort only. Sequencer admission still authenticates
                    // and reserves gas before accepting the transaction.
                    return Ok(());
                }
            };
        match Self::request_affordability_preflight(
            &authenticated_tx,
            &auth_data,
            S::Address::from_vm_address(EthereumAddress::from(from)),
            request.value.unwrap_or_default(),
            &mut affordability_state,
            ethereum,
        )? {
            AffordabilityPreflight::Affordable | AffordabilityPreflight::Skip => Ok(()),
            AffordabilityPreflight::Rejected(err) => Err(err),
        }
    }

    pub(crate) fn request_affordability_preflight(
        authenticated_tx: &AuthenticatedTransactionData<S>,
        auth_data: &AuthorizationData<S>,
        sender_rollup_addr: S::Address,
        requested_value: U256,
        state: &mut ApiStateAccessor<S>,
        ethereum: &Arc<Ethereum<S, Seq>>,
    ) -> RpcResult<AffordabilityPreflight> {
        let gas_cost = U256::from(authenticated_tx.0.max_fee.0);
        if gas_cost.is_zero() && requested_value.is_zero() {
            return Ok(AffordabilityPreflight::Affordable);
        }

        let sender_balance = Self::read_balance(&sender_rollup_addr, state)?;

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
                // Wrapper preflight is best-effort only. Sequencer admission still authenticates
                // and reserves gas before accepting the transaction.
                return Ok(AffordabilityPreflight::Skip);
            }
        };

        let overflow_error = || {
            ErrorObjectOwned::from(EthApiError::InvalidTransaction(
                RpcInvalidTransactionError::GasUintOverflow,
            ))
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
            let total_cost = gas_cost
                .checked_add(requested_value)
                .ok_or_else(overflow_error)?;
            return Ok(AffordabilityPreflight::Rejected(
                Self::insufficient_funds_error(total_cost, sender_balance),
            ));
        }

        let actual_payer = *context.gas_refund_recipient();

        if actual_payer == sender_rollup_addr {
            // Sender pays gas + value.
            let total_cost = gas_cost
                .checked_add(requested_value)
                .ok_or_else(overflow_error)?;
            if sender_balance < total_cost {
                return Ok(AffordabilityPreflight::Rejected(
                    Self::insufficient_funds_error(total_cost, sender_balance),
                ));
            }
        } else {
            // Paymaster covers gas (validated by try_reserve_gas). Sender covers value only.
            if sender_balance < requested_value {
                return Ok(AffordabilityPreflight::Rejected(ErrorObjectOwned::from(
                    EthApiError::InvalidTransaction(
                        RpcInvalidTransactionError::InsufficientFundsForTransfer,
                    ),
                )));
            }
        }

        Ok(AffordabilityPreflight::Affordable)
    }
}
