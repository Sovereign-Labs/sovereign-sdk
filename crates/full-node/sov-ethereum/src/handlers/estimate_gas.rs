use crate::handlers::Handlers;
use crate::{rpc_internal_error, Ethereum};
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
use sov_modules_api::capabilities::GasEnforcer;
use sov_modules_api::capabilities::TransactionAuthorizer;
use sov_modules_api::capabilities::{AuthorizationData, HasCapabilities, HasKernel};
use sov_modules_api::{
    ApiStateAccessor, AuthenticatedTransactionData, ExecutionContext, GetGasPrice, Spec,
};
use sov_rpc_eth_types::{EthApiError, RpcInvalidTransactionError};
use sov_sequencer::Sequencer;
use std::sync::Arc;

pub(crate) enum AffordabilityPreflight {
    Affordable,
    Rejected(ErrorObjectOwned),
    Skip,
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
        && request.max_fee_per_gas.or(request.gas_price).is_some()
        && state_overrides.is_none()
        && block_overrides.is_none()
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

        let should_run_affordability_preflight = supports_request_affordability_preflight(
            &request,
            state_overrides.as_ref(),
            block_overrides.as_deref(),
        );

        Self::validate_request_stale_nonce_preflight(&request, block_id, snapshot_state)?;

        let has_explicit_gas = request.gas.is_some();
        if has_explicit_gas && should_run_affordability_preflight {
            Self::run_request_affordability_preflight(
                &request,
                block_id,
                snapshot_state,
                ethereum,
            )?;
        }

        let mut state = snapshot_state.clone_without_local_writes();
        let estimated_gas = evm.eth_estimate_gas_helper(
            request.clone(),
            block_id,
            state_overrides,
            block_overrides,
            &mut state,
        )?;

        if !has_explicit_gas && should_run_affordability_preflight {
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

    pub(crate) fn run_request_affordability_preflight(
        request: &TransactionRequest,
        block_id: Option<BlockId>,
        snapshot_state: &ApiStateAccessor<S>,
        ethereum: &Arc<Ethereum<S, Seq>>,
    ) -> RpcResult<()> {
        let evm = Evm::<S>::default();
        let mut api_state = snapshot_state.clone_without_local_writes();
        let preflight_state = evm
            .preflight_state_for_block_id(block_id, &mut api_state)
            .map_err(ErrorObjectOwned::from)?;
        let mut affordability_state = preflight_state.clone_without_local_writes();

        // These guards are defensive: omitted-gas callers synthesize `gas` from a pinned estimate
        // before calling this helper, and the caller already validates `from` and a fee field.
        let Some(from) = request.from else {
            return Ok(());
        };
        if request.gas.is_none() {
            return Ok(());
        }
        if request.max_fee_per_gas.or(request.gas_price).is_none() {
            return Ok(());
        }

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
        let evm = Evm::<S>::default();
        let fee_check_active = evm
            .is_max_fee_check_active(state)
            .map_err(|e| rpc_internal_error(format!("state read error: {e}")))?;
        if !fee_check_active {
            return Ok(AffordabilityPreflight::Skip);
        }

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

#[cfg(test)]
mod tests {
    use super::TransactionRequest;
    use alloy_primitives::{Address, TxKind};

    fn sample_affordability_request() -> TransactionRequest {
        TransactionRequest {
            from: Some(Address::repeat_byte(0x11)),
            to: Some(TxKind::Call(Address::repeat_byte(0x22))),
            max_fee_per_gas: Some(1),
            ..Default::default()
        }
    }

    #[test]
    fn omitted_gas_requests_still_support_affordability_preflight() {
        let request = sample_affordability_request();

        assert!(super::supports_request_affordability_preflight(
            &request, None, None,
        ));
    }

    #[test]
    fn overrides_disable_affordability_preflight() {
        let request = sample_affordability_request();
        let state_overrides = alloy_rpc_types::state::StateOverride::default();

        assert!(!super::supports_request_affordability_preflight(
            &request,
            Some(&state_overrides),
            None,
        ));
    }
}
