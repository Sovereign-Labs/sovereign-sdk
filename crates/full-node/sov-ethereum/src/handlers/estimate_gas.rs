use crate::handlers::Handlers;
use crate::{rpc_internal_error, Ethereum};
use alloy_consensus::{EthereumTxEnvelope, Signed, TxEip1559};
use alloy_eips::BlockId;
use alloy_eips::Encodable2718;
use alloy_primitives::{Address, Signature, TxKind, B256, U256, U64};
use alloy_rpc_types::state::StateOverride;
use alloy_rpc_types::{BlockOverrides, TransactionRequest};
use jsonrpsee::core::RpcResult;
use jsonrpsee::types::ErrorObjectOwned;
use jsonrpsee::types::Params as JRpcParams;
use jsonrpsee::Extensions;
use sov_address::{EthereumAddress, FromVmAddress};
use sov_evm::{
    build_request_preflight_auth, EthereumAuthenticator, Evm, RlpEvmTransaction, TransactionSigned,
};
use sov_modules_api::capabilities::GasEnforcer;
use sov_modules_api::capabilities::{
    AuthorizationData, HasCapabilities, HasKernel, TransactionAuthenticator, TransactionAuthorizer,
    UniquenessData,
};
use sov_modules_api::macros::config_value;
use sov_modules_api::{
    ApiStateAccessor, AuthenticatedTransactionData, BasicGasMeter, DispatchCall, ExecutionContext,
    Gas, GasMeter, GasSpec, GetGasPrice, RawTx, Runtime, Spec, StateProvider as _, TxHooks,
    WorkingSet,
};
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

        let estimate_with_legacy = || {
            let mut state = snapshot_state.clone_without_local_writes();
            evm.eth_estimate_gas_helper(
                request.clone(),
                block_id,
                state_overrides.clone(),
                block_overrides.clone(),
                &mut state,
            )
        };
        let estimated_gas = if Self::should_use_runtime_parity_estimate(&request, has_overrides) {
            match Self::estimate_gas_via_runtime_pipeline(
                request.clone(),
                block_id,
                snapshot_state,
                ethereum,
            ) {
                Ok(estimated_gas) => estimated_gas,
                Err(err) => {
                    tracing::warn!(
                        error = %err,
                        "runtime-parity estimateGas failed; falling back to legacy estimator"
                    );
                    estimate_with_legacy()?
                }
            }
        } else {
            estimate_with_legacy()?
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

    fn should_use_runtime_parity_estimate(
        request: &TransactionRequest,
        has_overrides: bool,
    ) -> bool {
        !has_overrides
            && request.from.is_some()
            && request.max_fee_per_gas.or(request.gas_price).is_some()
    }

    fn estimate_gas_via_runtime_pipeline(
        request: TransactionRequest,
        block_id: Option<BlockId>,
        snapshot_state: &ApiStateAccessor<S>,
        ethereum: &Arc<Ethereum<S, Seq>>,
    ) -> Result<U64, String> {
        let evm = Evm::<S>::default();
        let mut api_state = snapshot_state.clone_without_local_writes();
        let mut preflight_state = evm
            .preflight_state_for_block_id(block_id, &mut api_state)
            .map_err(|err| format!("preflight state error: {err}"))?;
        let block_env = evm
            .block_env(&mut preflight_state)
            .map_err(|err| format!("block env read error: {err}"))?;
        let cfg = evm
            .cfg(&mut preflight_state)
            .map_err(|err| format!("cfg read error: {err}"))?;

        let mut normalized_request = request;
        Self::normalize_request_for_runtime(
            &mut normalized_request,
            block_env.gas_limit,
            cfg.chain_spec.tx_gas_limit,
            preflight_state.gas_price().as_ref()[0].0,
        );

        let mut auth_state = preflight_state
            .clone_without_local_writes()
            .to_provable_reader();
        let (authenticated_tx, auth_data) =
            build_request_preflight_auth::<_, S>(&normalized_request, &mut auth_state)
                .map_err(|err| format!("request preflight auth error: {err}"))?;
        let (tx_hash, runtime_call) =
            Self::build_runtime_estimate_call(normalized_request, &auth_data)?;

        let mut runtime = Seq::Rt::default();
        let gas_price = preflight_state.gas_price();
        let mut pre_exec_working_set = preflight_state.to_tx_scratchpad().to_pre_exec_working_set(
            BasicGasMeter::new_with_gas(<S as GasSpec>::max_tx_check_costs(), gas_price),
        );
        pre_exec_working_set
            .charge_gas(<S as GasSpec>::process_tx_pre_exec_checks_gas())
            .map_err(|err| format!("pre-exec gas charge failed: {err}"))?;

        let execution_context = ExecutionContext::Sequencer;
        let mut context = runtime
            .transaction_authorizer()
            .resolve_context(
                &auth_data,
                &ethereum.sequencer_da_address,
                ethereum.sequencer_rollup_address,
                &mut pre_exec_working_set,
                None,
                execution_context,
                ethereum.sequencer_type,
            )
            .map_err(|err| format!("resolve_context failed: {err}"))?;

        runtime
            .transaction_authorizer()
            .check_uniqueness(
                &auth_data,
                &context,
                &execution_context,
                &mut pre_exec_working_set,
            )
            .map_err(|err| format!("check_uniqueness failed: {err}"))?;

        runtime
            .transaction_authorizer()
            .mark_tx_attempted(
                &auth_data,
                &ethereum.sequencer_da_address,
                &mut pre_exec_working_set,
            )
            .map_err(|err| format!("mark_tx_attempted failed: {err}"))?;

        let gas_price = pre_exec_working_set.gas_price();
        runtime
            .gas_enforcer()
            .try_reserve_gas(
                &authenticated_tx,
                gas_price,
                &mut context,
                &mut pre_exec_working_set,
            )
            .map_err(|err| format!("try_reserve_gas failed: {err}"))?;

        let (scratchpad, pre_exec_gas_meter) = pre_exec_working_set.to_scratchpad_and_gas_meter();
        let mut working_set = WorkingSet::create_working_set(
            scratchpad,
            &authenticated_tx,
            authenticated_tx.gas_meter(pre_exec_gas_meter.gas_info().gas_price, <S::Gas>::max()),
        );

        working_set
            .charge_gas(pre_exec_gas_meter.gas_info().gas_used)
            .map_err(|err| format!("charging pre-exec gas failed: {err}"))?;

        if runtime.is_unauthorized_system_tx(&runtime_call, &context, &mut working_set) {
            return Err("unauthorized system transaction".to_string());
        }

        runtime
            .pre_dispatch_tx_hook(&authenticated_tx, &mut working_set)
            .map_err(|err| format!("pre_dispatch_tx_hook failed: {err}"))?;
        runtime
            .dispatch_call(runtime_call, &mut working_set, &context)
            .map_err(|err| format!("dispatch_call failed: {err}"))?;
        runtime
            .post_dispatch_tx_hook(&authenticated_tx, &context, &mut working_set)
            .map_err(|err| format!("post_dispatch_tx_hook failed: {err}"))?;

        let (tx_scratchpad, _, _) = working_set.finalize();
        let mut parity_state = tx_scratchpad.commit();
        let tx_index = evm
            .tx_index(&tx_hash, &mut parity_state)
            .ok_or_else(|| "no EVM transaction index produced by parity estimate".to_string())?;
        let (receipt, _) = evm
            .receipt(tx_index, &mut parity_state)
            .ok_or_else(|| "no EVM receipt produced by parity estimate".to_string())?;

        Ok(U64::from(receipt.gas_used))
    }

    fn normalize_request_for_runtime(
        request: &mut TransactionRequest,
        block_gas_limit: u64,
        tx_gas_limit: Option<u64>,
        default_gas_price: u128,
    ) {
        if request.from.is_none() {
            request.from = Some(Address::ZERO);
        }

        if request.gas.is_none() {
            request.gas = Some(block_gas_limit.min(tx_gas_limit.unwrap_or(block_gas_limit)));
        }

        if request.max_fee_per_gas.is_none() && request.gas_price.is_none() {
            request.gas_price = Some(default_gas_price);
        }

        if request.chain_id.is_none() {
            request.chain_id = Some(config_value!("CHAIN_ID"));
        }
    }

    fn build_runtime_estimate_call(
        request: TransactionRequest,
        auth_data: &AuthorizationData<S>,
    ) -> Result<(B256, <Seq::Rt as DispatchCall>::Decodable), String> {
        let nonce = match auth_data.uniqueness {
            UniquenessData::Nonce(nonce) => nonce,
            other => {
                return Err(format!(
                    "unexpected uniqueness type for EVM estimate: {other:?}"
                ));
            }
        };
        request
            .from
            .ok_or_else(|| "normalized request missing from".to_string())?;
        let gas_limit = request
            .gas
            .ok_or_else(|| "normalized request missing gas".to_string())?;
        let max_fee_per_gas = request
            .max_fee_per_gas
            .or(request.gas_price)
            .ok_or_else(|| "normalized request missing fee field".to_string())?;
        let input = request.input.clone().into_input().unwrap_or_default();
        let tx = TxEip1559 {
            chain_id: request.chain_id.unwrap_or(config_value!("CHAIN_ID")),
            nonce,
            gas_limit,
            max_fee_per_gas,
            max_priority_fee_per_gas: request.max_priority_fee_per_gas.unwrap_or(0),
            to: request.to.unwrap_or(TxKind::Create),
            value: request.value.unwrap_or_default(),
            input,
            access_list: request.access_list.unwrap_or_default(),
        };
        let envelope: TransactionSigned = EthereumTxEnvelope::Eip1559(Signed::new_unchecked(
            tx,
            Signature::test_signature(),
            Default::default(),
        ));
        let tx_hash = *envelope.hash();
        let raw_tx = borsh::to_vec(&RlpEvmTransaction {
            rlp: envelope.encoded_2718(),
        })
        .map_err(|err| format!("borsh serialize synthetic tx failed: {err}"))?;
        let serialized_tx = Seq::Rt::encode_with_ethereum_auth(RawTx::new(raw_tx));
        let auth_call =
            <<Seq::Rt as Runtime<S>>::Auth as TransactionAuthenticator<S>>::decode_serialized_tx(
                &serialized_tx,
            )
            .map_err(|err| format!("decode_serialized_tx failed: {err}"))?;
        let runtime_call = Seq::Rt::wrap_call(auth_call);

        Ok((tx_hash, runtime_call))
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
