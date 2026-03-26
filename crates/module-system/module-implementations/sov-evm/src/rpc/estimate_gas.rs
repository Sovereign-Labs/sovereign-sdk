use crate::evm::primitive_types::{PendingTransaction, TxSignedAndRecovered};
use crate::rpc::error::into_rpc_error;
use crate::{
    build_request_preflight_auth, EthereumAuthenticator, Evm, PreparedRuntimeParityEstimate,
    RlpEvmTransaction, TransactionSigned,
};
use alloy_consensus::{EthereumTxEnvelope, Signed, TxEip1559};
use alloy_eips::{BlockId, Encodable2718};
use alloy_primitives::{Address, TxKind, U64};
use alloy_rpc_types::state::StateOverride;
use alloy_rpc_types::{BlockOverrides, TransactionRequest};
use jsonrpsee::core::RpcResult;
use revm::context::result::{ExecutionResult, ResultAndState};
use revm_database_interface::TryDatabaseCommit;
use sov_address::{EthereumAddress, FromVmAddress};
use sov_modules_api::capabilities::ChainState;
use sov_modules_api::capabilities::SequencingDataHandler;
use sov_modules_api::capabilities::TransactionAuthenticator;
use sov_modules_api::macros::config_value;
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::transaction::AuthenticatedTransactionAndRawHash;
use sov_modules_api::{
    ApiStateAccessor, DispatchCall, GasMeter, GasSpec, GetGasPrice, InfallibleStateReaderAndWriter,
    Runtime, SequencerType, Spec, StateAccessor, StateProvider,
};
use sov_rollup_interface::stf::RawTx;
use sov_rollup_interface::TxHash;
use sov_rpc_eth_types::{RevertError, RpcInvalidTransactionError};
use sov_state::User;
use tracing::trace;

impl<S: Spec> Evm<S>
where
    S::Address: FromVmAddress<EthereumAddress>,
{
    /// Runs a quick EVM simulation and returns `Err` with a proper revert
    /// error (including raw output bytes) if the transaction would revert.
    /// Returns `Ok(())` for successful calls.
    ///
    /// This is used as a pre-check before the runtime-parity STF pipeline,
    /// which loses revert output bytes during receipt construction.
    #[doc(hidden)]
    pub fn check_for_evm_revert(
        &self,
        request: &TransactionRequest,
        block_id: Option<BlockId>,
        state: &mut ApiStateAccessor<S>,
    ) -> RpcResult<()> {
        let ResultAndState { result, .. } =
            self.call(request.clone(), block_id, None, None, state)?;
        match result {
            ExecutionResult::Success { .. } => Ok(()),
            ExecutionResult::Revert { output, .. } => {
                Err(RpcInvalidTransactionError::Revert(RevertError::new(output)).into())
            }
            ExecutionResult::Halt { reason, gas_used } => {
                Err(RpcInvalidTransactionError::halt(reason, gas_used).into())
            }
        }
    }

    /// Runs gas estimation logic for `eth_estimateGas`.
    ///
    /// This is a library method called by `sov-ethereum`'s RPC handler, which
    /// wraps it with paymaster-aware affordability checks.
    pub fn eth_estimate_gas_helper(
        &self,
        request: TransactionRequest,
        block_id: Option<BlockId>,
        state_overrides: Option<StateOverride>,
        block_overrides: Option<Box<BlockOverrides>>,
        state: &mut ApiStateAccessor<S>,
    ) -> RpcResult<U64> {
        trace!(
            ?block_id,
            method = "eth_estimateGas",
            "EVM module JSON-RPC request"
        );
        let mut metered_state = state.clone_without_local_writes();
        let (block_env, maybe_archival_state, cfg) =
            self.resolve_simulation_context_for_block_id(block_id, state)?;
        let block_number = block_env.number.to::<u64>();

        let ResultAndState {
            result,
            state: changes,
        } = self.call_with_context(
            request.clone(),
            block_env.clone(),
            maybe_archival_state,
            &cfg,
            state_overrides,
            block_overrides,
        )?;

        let mut normalized_request = request;
        Self::normalize_runtime_parity_request(
            &mut normalized_request,
            block_env.gas_limit,
            cfg.chain_spec.tx_gas_limit,
            metered_state.gas_price().as_ref()[0].0,
        );
        Self::fill_current_nonce_if_missing(
            &self.uniqueness_module,
            &mut normalized_request,
            &mut metered_state,
        );
        let signer = normalized_request.from.unwrap_or_default();
        let synthetic_nonce = normalized_request
            .nonce
            .ok_or_else(|| into_rpc_error("normalized estimate_gas request missing nonce"))?;
        let synthetic_signed_tx =
            Self::build_runtime_parity_signed_tx(&normalized_request, synthetic_nonce)
                .map_err(into_rpc_error)?;

        let gas_used = match &result {
            ExecutionResult::Success { gas_used, .. } => *gas_used,
            ExecutionResult::Revert { output, .. } => {
                return Err(
                    RpcInvalidTransactionError::Revert(RevertError::new(output.clone())).into(),
                );
            }
            ExecutionResult::Halt { reason, gas_used } => {
                return Err(RpcInvalidTransactionError::halt(reason.clone(), *gas_used).into());
            }
        };

        // `metered_state` was cloned before block-resolution reads so receipt-style overhead
        // reflects only this request, not selector-resolution gas from `"pending"` vs numeric tags.
        self.db(&mut metered_state)
            .try_commit(changes)
            .expect("Gas meter is initialized with INF");

        let pending_len = self
            .pending_transactions
            .len(&mut metered_state)
            .unwrap_infallible();
        let synthetic_tx = TxSignedAndRecovered::new(signer, synthetic_signed_tx, block_number);
        let mut evm = self.clone();
        let receipt = evm
            .create_receipt(&synthetic_tx, pending_len, result, &mut metered_state)
            .map_err(|err| {
                into_rpc_error(format!("estimate_gas receipt reconstruction failed: {err}"))
            })?;

        let gas_meter = metered_state
            .try_as_basic_gas_meter()
            .expect("ApiState has BasicGasMeter");
        let gas_used =
            u32::try_from(gas_used).map_err(|_| RpcInvalidTransactionError::GasUintOverflow)?;
        gas_meter
            .charge_linear_gas(<S as GasSpec>::gas_to_charge_per_evm_gas(), gas_used)
            .expect("Gas meter is initialized with INF");
        let time = evm
            .chain_state_module
            .get_oracle_time(&mut metered_state)
            .unwrap_infallible();
        let mut pending_tx = PendingTransaction::new(synthetic_tx, receipt, time);

        let mut pending_transactions = self.pending_transactions.clone();
        pending_transactions
            .push(&pending_tx, &mut metered_state)
            .unwrap_infallible();
        let head = self
            .head
            .get(&mut metered_state)
            .unwrap_infallible()
            .expect("Head is set in genesis and never deleted");

        let gas_info = metered_state
            .try_as_basic_gas_meter()
            .expect("ApiState has BasicGasMeter")
            .gas_info();
        if let Some(projected_gas) =
            crate::sov_fee_and_gas_utils::project_receipt_gas_from_actual_fee::<S>(
                &pending_tx.receipt,
                &gas_info,
            )
            .map_err(into_rpc_error)?
        {
            pending_tx.receipt.gas_used = projected_gas.gas_used;
            pending_tx.receipt.receipt.cumulative_gas_used = projected_gas.cumulative_gas_used;

            let mut unmetered_state = metered_state.to_unmetered();
            pending_transactions
                .set(pending_len, &pending_tx, &mut unmetered_state)
                .unwrap_infallible()
                .map_err(into_rpc_error)?;
        }

        evm.set_accessory_state(
            head,
            &pending_tx,
            pending_len + 1,
            gas_info.gas_value,
            &mut metered_state,
        )
        .unwrap_infallible();

        Ok(U64::from(pending_tx.receipt.gas_used))
    }

    /// Prepares the pinned pre-exec state and synthetic transaction needed to
    /// run the STF pipeline for runtime-parity gas estimation.
    #[doc(hidden)]
    pub fn prepare_runtime_parity_estimate<R>(
        &self,
        request: TransactionRequest,
        block_id: Option<BlockId>,
        snapshot_state: &ApiStateAccessor<S>,
        sequencer_type: SequencerType,
    ) -> Result<PreparedRuntimeParityEstimate<S, <R as DispatchCall>::Decodable>, String>
    where
        R: Runtime<S> + EthereumAuthenticator<S> + Default,
    {
        let mut api_state = snapshot_state.clone_without_local_writes();
        let mut block_env_state = snapshot_state.clone_without_local_writes();
        let mut preflight_state = self
            .preflight_state_for_block_id(block_id, &mut api_state)
            .map_err(|err| format!("preflight state error: {err}"))?;
        let block_env = self
            .resolve_block_env_for_call(block_id, &mut block_env_state)
            .map_err(|err| format!("block env read error: {err}"))?;
        let cfg = self
            .cfg(&mut preflight_state)
            .map_err(|err| format!("cfg read error: {err}"))?;

        let mut normalized_request = request;
        Self::normalize_runtime_parity_request(
            &mut normalized_request,
            block_env.gas_limit,
            cfg.chain_spec.tx_gas_limit,
            preflight_state.gas_price().as_ref()[0].0,
        );
        Self::fill_current_nonce_if_missing(
            &self.uniqueness_module,
            &mut normalized_request,
            &mut preflight_state,
        );

        let mut runtime = R::default();
        let sequencing_data = if sequencer_type == SequencerType::Preferred {
            Some(
                borsh::to_vec(&runtime.sequencing_data_handler().create_sequencing_data())
                    .map(sov_rollup_interface::Bytes::from)
                    .map_err(|err| format!("sequencing data serialization failed: {err}"))?,
            )
        } else {
            None
        };
        let mut operating_mode_state = preflight_state.clone_without_local_writes();
        let operating_mode = runtime
            .chain_state()
            .operating_mode(&mut operating_mode_state);
        let gas_price = preflight_state.gas_price();
        let mut pre_exec_working_set = preflight_state.to_tx_scratchpad().to_pre_exec_working_set(
            sov_modules_api::BasicGasMeter::new_with_gas(
                <S as GasSpec>::max_tx_check_costs(),
                gas_price,
            ),
        );
        pre_exec_working_set
            .charge_gas(<S as GasSpec>::process_tx_pre_exec_checks_gas())
            .map_err(|err| format!("pre-exec gas charge failed: {err}"))?;
        let (authenticated_tx, auth_data) =
            build_request_preflight_auth::<_, S>(&normalized_request, &mut pre_exec_working_set)
                .map_err(|err| format!("request preflight auth error: {err}"))?;
        let (raw_tx_hash, raw_tx, runtime_call) = Self::build_runtime_parity_baked_tx::<R>(
            normalized_request,
            &auth_data,
            sequencing_data,
        )?;

        Ok(PreparedRuntimeParityEstimate {
            pre_exec_working_set,
            authenticated_tx: AuthenticatedTransactionAndRawHash {
                raw_tx_hash,
                authenticated_tx,
            },
            auth_data,
            runtime_call,
            raw_tx,
            operating_mode,
        })
    }

    /// Reads the simulated gas estimate from the newest pending EVM receipt in
    /// the provided scratch state.
    #[doc(hidden)]
    pub fn read_runtime_parity_estimate_from_pending_tail<
        Accessor: InfallibleStateReaderAndWriter<User>,
    >(
        &self,
        state: &mut Accessor,
    ) -> Result<U64, String> {
        let pending_tx = self
            .pending_transactions
            .last(state)
            .unwrap_infallible()
            .ok_or_else(|| "no pending EVM transaction produced by parity estimate".to_string())?;

        // When `preferred_sequencer_publish_reverted_txs = true`, the SDK tx
        // succeeds even though the EVM tx reverted (`TxEffect::Successful`).
        // The receipt's `success` field is always set correctly by
        // `create_receipt()`, so we check it here to ensure `eth_estimateGas`
        // returns a revert error rather than a gas estimate.
        if !pending_tx.receipt.success {
            return Err("EVM transaction reverted".to_string());
        }

        Ok(U64::from(pending_tx.receipt.gas_used))
    }

    fn normalize_runtime_parity_request(
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

    fn fill_current_nonce_if_missing(
        uniqueness_module: &sov_uniqueness::Uniqueness<S>,
        request: &mut TransactionRequest,
        state: &mut ApiStateAccessor<S>,
    ) {
        if request.nonce.is_some() {
            return;
        }

        let Some(from) = request.from else {
            return;
        };
        let credential_id = EthereumAddress::from(from).as_credential_id();
        let mut nonce_state = state.clone_without_local_writes();
        let nonce = uniqueness_module
            .next_nonce(&credential_id, &mut nonce_state)
            .unwrap_or_default();
        request.nonce = Some(nonce);
    }

    fn runtime_parity_nonce(
        auth_data: &sov_modules_api::capabilities::AuthorizationData<S>,
    ) -> Result<u64, String> {
        match auth_data.uniqueness {
            sov_modules_api::capabilities::UniquenessData::Nonce(nonce) => Ok(nonce),
            other => Err(format!(
                "unexpected uniqueness type for EVM estimate: {other:?}"
            )),
        }
    }

    fn build_runtime_parity_signed_tx(
        request: &TransactionRequest,
        nonce: u64,
    ) -> Result<TransactionSigned, String> {
        let gas_limit = request
            .gas
            .ok_or_else(|| "normalized request missing gas".to_string())?;
        let max_fee_per_gas = request
            .max_fee_per_gas
            .or(request.gas_price)
            .ok_or_else(|| "normalized request missing fee field".to_string())?;
        let input = request.input.clone().into_input().unwrap_or_default();
        Ok(EthereumTxEnvelope::Eip1559(Signed::new_unchecked(
            TxEip1559 {
                chain_id: request.chain_id.unwrap_or(config_value!("CHAIN_ID")),
                nonce,
                gas_limit,
                max_fee_per_gas,
                max_priority_fee_per_gas: request.max_priority_fee_per_gas.unwrap_or(0),
                to: request.to.unwrap_or(TxKind::Create),
                value: request.value.unwrap_or_default(),
                input,
                access_list: request.access_list.clone().unwrap_or_default(),
            },
            alloy_primitives::Signature::test_signature(),
            Default::default(),
        )))
    }

    fn build_runtime_parity_baked_tx<R>(
        request: TransactionRequest,
        auth_data: &sov_modules_api::capabilities::AuthorizationData<S>,
        sequencing_data: Option<sov_rollup_interface::Bytes>,
    ) -> Result<
        (
            TxHash,
            sov_modules_api::FullyBakedTx,
            <R as DispatchCall>::Decodable,
        ),
        String,
    >
    where
        R: Runtime<S> + EthereumAuthenticator<S>,
    {
        let nonce = Self::runtime_parity_nonce(auth_data)?;
        let envelope = Self::build_runtime_parity_signed_tx(&request, nonce)?;
        let tx_hash = TxHash::new(**envelope.hash());
        let raw_tx = borsh::to_vec(&RlpEvmTransaction {
            rlp: envelope.encoded_2718(),
        })
        .map_err(|err| format!("borsh serialize synthetic tx failed: {err}"))?;
        let mut serialized_tx = R::encode_with_ethereum_auth(RawTx::new(raw_tx));
        serialized_tx.sequencing_data = sequencing_data;
        let auth_call =
            <R::Auth as TransactionAuthenticator<S>>::decode_serialized_tx(&serialized_tx)
                .map_err(|err| format!("decode_serialized_tx failed: {err}"))?;
        let runtime_call = R::wrap_call(auth_call);

        Ok((tx_hash, serialized_tx, runtime_call))
    }
}
