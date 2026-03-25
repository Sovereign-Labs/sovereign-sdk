use crate::error::into_rpc_error;
use crate::rpc::error::ensure_success;
use alloy_consensus::private::alloy_eips::Encodable2718;
use alloy_consensus::{EthereumTxEnvelope, ReceiptEnvelope, Signed, TxEip1559};
use alloy_eips::BlockId;
use alloy_primitives::{Address, TxKind, U64};
use alloy_primitives::{Bytes, B256, U256};
use alloy_rpc_types::{
    state::StateOverride, AccessListResult, Block, BlockNumberOrTag, BlockOverrides, FeeHistory,
    Transaction, TransactionReceipt, TransactionRequest,
};
use alloy_rpc_types_trace::geth::GethDebugTracingOptions;
use alloy_rpc_types_trace::geth::{GethTrace, TraceResult};
use jsonrpsee::core::RpcResult;
use revm::context::result::{ExecutionResult, ResultAndState};
use revm::Database;
use revm_database_interface::TryDatabaseCommit;
use revm_inspectors::access_list::AccessListInspector;
use sov_address::{EthereumAddress, FromVmAddress};
use sov_modules_api::capabilities::{ChainState, SequencingDataHandler, TransactionAuthenticator};
use sov_modules_api::macros::{config_value, rpc_gen};
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::transaction::AuthenticatedTransactionAndRawHash;
use sov_modules_api::{
    ApiStateAccessor, DispatchCall, GasMeter, GasSpec, GetGasPrice, InfallibleStateReaderAndWriter,
    RawTx, Runtime, SequencerType, Spec, StateAccessor, StateProvider as _,
};
use sov_rollup_interface::TxHash;
use sov_rpc_eth_types::{
    EthApiError, LogWithExecutionTimestamp, RevertError, RpcInvalidTransactionError,
};
use sov_state::User;
use std::ops::DerefMut;
use tracing::trace;

use crate::evm::primitive_types::{PendingTransaction, TxSignedAndRecovered};
use crate::{
    build_request_preflight_auth, EthereumAuthenticator, Evm, PreparedRuntimeParityEstimate,
    RlpEvmTransaction, TransactionSigned,
};

#[rpc_gen(client, server)]
impl<S: Spec> Evm<S>
where
    S::Address: FromVmAddress<EthereumAddress>,
{
    /// Handler for `net_version`
    #[rpc_method(name = "net_version")]
    pub fn net_version(&self, _state: &mut ApiStateAccessor<S>) -> RpcResult<String> {
        trace!(method = "net_version", "EVM module JSON-RPC request");

        // Network ID is the same as chain ID for most networks
        let chain_id: u64 = config_value!("CHAIN_ID");
        Ok(chain_id.to_string())
    }

    /// Handler for: `eth_chainId`
    #[rpc_method(name = "eth_chainId")]
    pub fn chain_id(&self, _state: &mut ApiStateAccessor<S>) -> RpcResult<Option<U64>> {
        let chain_id: u64 = config_value!("CHAIN_ID");
        trace!(
            chain_id = chain_id,
            method = "eth_chainId",
            "EVM module JSON-RPC request"
        );
        Ok(Some(U64::from(chain_id)))
    }

    /// Handler for `eth_getBlockByHash`
    #[rpc_method(name = "eth_getBlockByHash")]
    pub fn get_block_by_hash(
        &self,
        block_hash: B256,
        details: Option<bool>,
        state: &mut ApiStateAccessor<S>,
    ) -> RpcResult<Option<Block>> {
        trace!(
            ?block_hash,
            method = "eth_getBlockByHash",
            "EVM module JSON-RPC request"
        );
        Ok(self.get_maybe_synthetic_block_for_rpc(
            Some(BlockId::Hash(block_hash.into())),
            details.unwrap_or_default().into(),
            state,
        )?)
    }

    /// Handler for: `eth_getBlockByNumber`
    #[rpc_method(name = "eth_getBlockByNumber")]
    pub fn get_block_by_number(
        &self,
        block_id: Option<BlockId>,
        details: Option<bool>,
        state: &mut ApiStateAccessor<S>,
    ) -> RpcResult<Option<Block>> {
        trace!(
            ?block_id,
            method = "eth_getBlockByNumber",
            "EVM module JSON-RPC request"
        );
        let kind = details.unwrap_or_default().into();
        Ok(self.get_maybe_synthetic_block_for_rpc(block_id, kind, state)?)
    }

    /// Handler for: `eth_getBalance`
    #[rpc_method(name = "eth_getBalance")]
    pub fn get_balance(
        &self,
        address: Address,
        block_id: Option<BlockId>,
        state: &mut ApiStateAccessor<S>,
    ) -> RpcResult<U256> {
        let mut state = self.resolve_state_for_block_id(block_id, state)?;
        let balance = self
            .db(state.deref_mut())
            .basic(address)
            .map_err(EthApiError::from)?
            .map(|account| account.balance)
            .unwrap_or_default();

        trace!(
            %address,
            %balance,
            method = "eth_getBalance",
            "EVM module JSON-RPC request"
        );

        Ok(balance)
    }

    /// Handler for: `eth_getStorageAt`
    #[rpc_method(name = "eth_getStorageAt")]
    pub fn get_storage_at(
        &self,
        address: Address,
        index: U256,
        block_id: Option<BlockId>,
        state: &mut ApiStateAccessor<S>,
    ) -> RpcResult<B256> {
        trace!(method = "eth_getStorageAt", ?block_id, %address, %index, "EVM module JSON-RPC request");

        let mut state = self.resolve_state_for_block_id(block_id, state)?;
        let storage_slot = self
            .account_storage
            .get(&(&address, &index), state.deref_mut())
            .unwrap_infallible()
            .unwrap_or_default();

        Ok(storage_slot.to_be_bytes::<32>().into())
    }

    /// Handler for: `eth_getTransactionCount`
    #[rpc_method(name = "eth_getTransactionCount")]
    pub fn get_transaction_count(
        &self,
        address: Address,
        block_id: Option<BlockId>,
        state: &mut ApiStateAccessor<S>,
    ) -> RpcResult<U64> {
        let mut state = self.resolve_state_for_block_id(block_id, state)?;

        let ethereum_address: EthereumAddress = address.into();
        let credential_id = ethereum_address.as_credential_id();

        let nonce = self
            .uniqueness_module
            .next_nonce(&credential_id, state.deref_mut())
            .unwrap_or_default();

        trace!(%address, nonce, method = "eth_getTransactionCount", "EVM module JSON-RPC request");
        Ok(U64::from(nonce))
    }

    /// Handler for: `eth_getCode`
    #[rpc_method(name = "eth_getCode")]
    pub fn get_code(
        &self,
        address: Address,
        block_id: Option<BlockId>,
        state: &mut ApiStateAccessor<S>,
    ) -> RpcResult<Bytes> {
        trace!(method = "eth_getCode", %address, ?block_id, "EVM module JSON-RPC request");
        let state = self.resolve_state_for_block_id(block_id, state)?;
        Ok(self.get_contract_code(address, state).unwrap_or_default())
    }

    /// Handler for: `eth_gasPrice`
    #[rpc_method(name = "eth_gasPrice")]
    pub fn gas_price(&self, state: &mut ApiStateAccessor<S>) -> RpcResult<U256> {
        trace!(method = "eth_gasPrice", "EVM module JSON-RPC request");
        Ok(U256::from(self.block_env(state)?.basefee))
    }

    /// Handler for: `eth_feeHistory`
    /// Returns historical gas price and usage data for recent blocks.
    ///
    /// This endpoint helps wallets and users determine appropriate gas prices
    /// by exposing the rollup's EIP-1559 style base fee history.
    #[rpc_method(name = "eth_feeHistory")]
    pub fn fee_history(
        &self,
        block_count: U64,
        newest_block: BlockNumberOrTag,
        reward_percentiles: Option<Vec<f64>>,
        state: &mut ApiStateAccessor<S>,
    ) -> RpcResult<FeeHistory> {
        let block_count = block_count.to::<u64>();
        trace!(
            block_count,
            ?newest_block,
            ?reward_percentiles,
            method = "eth_feeHistory",
            "EVM module JSON-RPC request"
        );

        Ok(self.get_fee_history(
            block_count,
            newest_block,
            reward_percentiles.as_deref(),
            state,
        )?)
    }

    /// Handler for: `eth_getTransactionByHash`
    #[rpc_method(name = "eth_getTransactionByHash")]
    pub fn get_transaction_by_hash(
        &self,
        hash: B256,
        state: &mut ApiStateAccessor<S>,
    ) -> RpcResult<Option<Transaction>> {
        let transaction = self.get_transaction(hash, state);
        trace!(
            %hash,
            ?transaction,
            method = "eth_getTransactionByHash",
            "EVM module JSON-RPC request"
        );
        Ok(transaction)
    }

    /// Handler for: `eth_getTransactionByBlockHashAndIndex`
    #[rpc_method(name = "eth_getTransactionByBlockHashAndIndex")]
    pub fn get_transaction_by_block_hash_and_index(
        &self,
        block_hash: B256,
        index: U64,
        state: &mut ApiStateAccessor<S>,
    ) -> RpcResult<Option<Transaction>> {
        trace!(
            %block_hash,
            index = index.to::<u64>(),
            method = "eth_getTransactionByBlockHashAndIndex",
            "EVM module JSON-RPC request"
        );
        let maybe_block =
            match self.get_maybe_sealed_block_by_id(BlockId::Hash(block_hash.into()), state) {
                Ok(block) => block,
                Err(EthApiError::HeaderNotFound(_)) => return Ok(None),
                Err(err) => return Err(err.into()),
            };
        let Some(block) = maybe_block else {
            return Ok(None);
        };
        get_transaction_for_block_index(self, block, index.to::<u64>(), state).map_err(Into::into)
    }

    /// Handler for: `eth_getTransactionByBlockNumberAndIndex`
    #[rpc_method(name = "eth_getTransactionByBlockNumberAndIndex")]
    pub fn get_transaction_by_block_number_and_index(
        &self,
        block: BlockNumberOrTag,
        index: U64,
        state: &mut ApiStateAccessor<S>,
    ) -> RpcResult<Option<Transaction>> {
        trace!(
            ?block,
            index = index.to::<u64>(),
            method = "eth_getTransactionByBlockNumberAndIndex",
            "EVM module JSON-RPC request"
        );
        let maybe_block = self.get_maybe_sealed_block_by_id(BlockId::Number(block), state)?;
        let Some(block) = maybe_block else {
            return Ok(None);
        };
        get_transaction_for_block_index(self, block, index.to::<u64>(), state).map_err(Into::into)
    }

    /// Handler for: `eth_getBlockReceipts`
    #[rpc_method(name = "eth_getBlockReceipts")]
    pub fn get_block_receipts(
        &self,
        block_id: Option<BlockId>,
        state: &mut ApiStateAccessor<S>,
    ) -> RpcResult<Option<Vec<TransactionReceipt<ReceiptEnvelope<LogWithExecutionTimestamp>>>>>
    {
        trace!(
            ?block_id,
            method = "eth_getBlockReceipts",
            "EVM module JSON-RPC request"
        );
        Ok(self.get_receipts(block_id, state)?)
    }

    /// Handler for: `eth_getTransactionReceipt`
    #[rpc_method(name = "eth_getTransactionReceipt")]
    pub fn get_transaction_receipt(
        &self,
        hash: B256,
        state: &mut ApiStateAccessor<S>,
    ) -> RpcResult<Option<TransactionReceipt<ReceiptEnvelope<LogWithExecutionTimestamp>>>> {
        trace!(
            %hash,
            method = "eth_getTransactionReceipt",
            "EVM module JSON-RPC request"
        );
        Ok(self.get_receipt_by_hash(hash, state))
    }

    /// Handler for: `eth_call`
    //https://github.com/paradigmxyz/reth/blob/f577e147807a783438a3f16aad968b4396274483/crates/rpc/rpc/src/eth/api/transactions.rs#L502
    #[rpc_method(name = "eth_call")]
    pub fn eth_call(
        &self,
        request: TransactionRequest,
        block_id: Option<BlockId>,
        state_overrides: Option<StateOverride>,
        block_overrides: Option<Box<BlockOverrides>>,
        state: &mut ApiStateAccessor<S>,
    ) -> RpcResult<Bytes> {
        trace!(
            method = "eth_call",
            ?block_id,
            "EVM module JSON-RPC request"
        );

        let result = self
            .call(request, block_id, state_overrides, block_overrides, state)?
            .result;
        Ok(ensure_success(result)?)
    }

    /// Handler for: `eth_createAccessList`
    #[rpc_method(name = "eth_createAccessList")]
    pub fn eth_create_access_list(
        &self,
        request: TransactionRequest,
        block_id: Option<BlockId>,
        state: &mut ApiStateAccessor<S>,
    ) -> RpcResult<AccessListResult> {
        trace!(
            ?block_id,
            method = "eth_createAccessList",
            "EVM module JSON-RPC request"
        );
        let initial_access_list = request.access_list.clone().unwrap_or_default();
        // Intentionally do not apply the shared simulation max-fee-vs-base-fee
        // rejection here. Geth's access-list path executes with `NoBaseFee: true`
        // and lowers the EVM base fee to `0` when needed, keeping low-fee
        // requests admissible during access-list generation:
        // https://github.com/ethereum/go-ethereum/blob/16783c167c4be5e6675fd8de0d1b762c88d6232f/internal/ethapi/api.go#L1359-L1372
        super::validate_call_fee_fields(&request)?;
        let (block_env, mut maybe_archival_state, cfg) =
            self.resolve_simulation_context_for_block_id(block_id, state)?;
        let tx_env =
            crate::helpers::prepare_call_env(&block_env, request, cfg.chain_spec.tx_gas_limit)?;
        let cfg_env =
            crate::executor::get_cfg_env(&block_env, &cfg, Some(super::get_cfg_env_template()));
        let evm_db = self.db(maybe_archival_state.deref_mut());

        let mut inspector = AccessListInspector::new(initial_access_list);
        let execution =
            crate::executor::inspect(evm_db, &block_env, tx_env, cfg_env, &mut inspector)
                .map_err(EthApiError::from)?;

        let (gas_used, error) = match execution.result {
            ExecutionResult::Success { gas_used, .. } => (U256::from(gas_used), None),
            ExecutionResult::Revert { gas_used, .. } => {
                (U256::from(gas_used), Some("execution reverted".to_string()))
            }
            ExecutionResult::Halt { gas_used, reason } => {
                (U256::from(gas_used), Some(format!("{reason:?}")))
            }
        };

        Ok(AccessListResult {
            access_list: inspector.into_access_list(),
            gas_used,
            error,
        })
    }

    /// Handler for: `eth_blockNumber`.
    /// Returns pending block if it has any transactions.
    /// This is in line with sovereign rollup `pending` == `latest` semantics.
    #[rpc_method(name = "eth_blockNumber")]
    pub fn block_number(&self, state: &mut ApiStateAccessor<S>) -> RpcResult<U256> {
        trace!(method = "eth_blockNumber", "EVM module JSON-RPC request");
        Ok(U256::from(
            self.resolve_block_number(BlockNumberOrTag::Latest, state),
        ))
    }

    /// Handler for `debug_traceBlockByNumber`
    #[rpc_method(name = "debug_traceBlockByNumber")]
    pub fn debug_trace_block_by_number(
        &self,
        block: BlockNumberOrTag,
        opts: Option<GethDebugTracingOptions>,
        state: &mut ApiStateAccessor<S>,
    ) -> RpcResult<Vec<TraceResult>> {
        trace!(
            method = "debug_traceBlockByNumber",
            "EVM module JSON-RPC request"
        );
        Ok(self.trace_block_by_number(block, opts.unwrap_or_default(), state)?)
    }

    /// Handler for: `debug_traceTransaction`
    #[rpc_method(name = "debug_traceTransaction")]
    pub fn debug_trace_transaction(
        &self,
        tx_hash: B256,
        opts: Option<GethDebugTracingOptions>,
        state: &mut ApiStateAccessor<S>,
    ) -> RpcResult<GethTrace> {
        trace!(method = "debug_traceTransaction", %tx_hash, "EVM module JSON-RPC request");
        Ok(self.trace_transaction(tx_hash, opts.unwrap_or_default(), state)?)
    }

    // ========== web3 namespace ==========

    /// Handler for: `web3_clientVersion`
    /// Returns the current client version.
    #[rpc_method(name = "web3_clientVersion")]
    pub fn web3_client_version(&self, _state: &mut ApiStateAccessor<S>) -> RpcResult<String> {
        trace!(method = "web3_clientVersion", "EVM module JSON-RPC request");
        Ok(format!(
            "sov-evm/{}",
            option_env!("CARGO_PKG_VERSION").unwrap_or("unknown")
        ))
    }

    /// Handler for: `web3_sha3`
    /// Returns Keccak-256 hash of the given data.
    #[rpc_method(name = "web3_sha3")]
    pub fn web3_sha3(&self, data: Bytes, _state: &mut ApiStateAccessor<S>) -> RpcResult<B256> {
        trace!(method = "web3_sha3", "EVM module JSON-RPC request");
        Ok(alloy_primitives::keccak256(&data))
    }

    // ========== net namespace ==========

    /// Handler for: `net_listening`
    /// Returns true if client is actively listening for network connections.
    #[rpc_method(name = "net_listening")]
    pub fn net_listening(&self, _state: &mut ApiStateAccessor<S>) -> RpcResult<bool> {
        trace!(method = "net_listening", "EVM module JSON-RPC request");
        // Rollup is always accepting connections via RPC
        Ok(true)
    }

    /// Handler for: `eth_maxPriorityFeePerGas`
    /// Returns the current max priority fee per gas.
    #[rpc_method(name = "eth_maxPriorityFeePerGas")]
    pub fn eth_max_priority_fee_per_gas(
        &self,
        _state: &mut ApiStateAccessor<S>,
    ) -> RpcResult<U256> {
        trace!(
            method = "eth_maxPriorityFeePerGas",
            "EVM module JSON-RPC request"
        );
        // Rollup uses preferred sequencer model, no priority fees
        Ok(U256::ZERO)
    }

    /// Handler for: `eth_getBlockTransactionCountByNumber`
    /// Returns the number of transactions in a block by block number.
    #[rpc_method(name = "eth_getBlockTransactionCountByNumber")]
    pub fn get_block_transaction_count_by_number(
        &self,
        block_id: Option<BlockId>,
        state: &mut ApiStateAccessor<S>,
    ) -> RpcResult<Option<U64>> {
        trace!(
            ?block_id,
            method = "eth_getBlockTransactionCountByNumber",
            "EVM module JSON-RPC request"
        );
        let block = self.get_maybe_synthetic_block_for_rpc(block_id, false.into(), state)?;
        Ok(block.map(|b| U64::from(b.transactions.len())))
    }

    /// Handler for: `eth_getBlockTransactionCountByHash`
    /// Returns the number of transactions in a block by block hash.
    #[rpc_method(name = "eth_getBlockTransactionCountByHash")]
    pub fn get_block_transaction_count_by_hash(
        &self,
        block_hash: B256,
        state: &mut ApiStateAccessor<S>,
    ) -> RpcResult<Option<U64>> {
        trace!(
            %block_hash,
            method = "eth_getBlockTransactionCountByHash",
            "EVM module JSON-RPC request"
        );
        let maybe_block =
            match self.get_maybe_sealed_block_by_id(BlockId::Hash(block_hash.into()), state) {
                Ok(block) => block,
                // For synthetic hashes that are not in cache, this endpoint should behave
                // like unknown block hash and return `null` instead of an RPC error.
                Err(EthApiError::HeaderNotFound(_)) => return Ok(None),
                Err(err) => return Err(err.into()),
            };

        Ok(maybe_block.map(|block| {
            U64::from(
                block
                    .transactions_end()
                    .saturating_sub(block.transactions_start()),
            )
        }))
    }
}

/// Methods that are NOT auto-registered via `#[rpc_gen]`.
/// `eth_estimate_gas` is registered by `sov-ethereum` which wraps it with
/// paymaster-aware affordability checks.
impl<S: Spec> Evm<S>
where
    S::Address: FromVmAddress<EthereumAddress>,
{
    /// Validates request-shape and base-fee rules for `eth_estimateGas` before any
    /// transport-specific affordability checks run.
    pub fn validate_estimate_gas_request(
        &self,
        request: &TransactionRequest,
        block_id: Option<BlockId>,
        state_overrides: Option<StateOverride>,
        block_overrides: Option<Box<BlockOverrides>>,
        state: &mut ApiStateAccessor<S>,
    ) -> RpcResult<()> {
        let has_overrides = state_overrides.is_some() || block_overrides.is_some();
        let block_env = self.resolve_block_env_for_call(block_id, state)?;
        let mut maybe_archival_state = self.resolve_state_for_block_id(block_id, state)?;
        let enforce_max_fee_check = self
            .is_max_fee_check_active(maybe_archival_state.deref_mut())
            .map_err(|e| EthApiError::other(into_rpc_error(e)))?;
        super::validate_call_fee_fields(request)?;

        if has_overrides {
            let mut validation_block_env = block_env.clone();
            {
                let evm_db = self.db(maybe_archival_state.deref_mut());
                let mut validation_state = revm::database::State::builder()
                    .with_database(evm_db)
                    .build();
                super::apply_call_overrides(
                    &mut validation_state,
                    &mut validation_block_env,
                    state_overrides,
                    block_overrides,
                )?;
            }
            super::validate_simulation_max_fee_against_base_fee(
                request,
                &validation_block_env,
                enforce_max_fee_check,
            )?;
        } else {
            super::validate_simulation_max_fee_against_base_fee(
                request,
                &block_env,
                enforce_max_fee_check,
            )?;
        }

        Ok(())
    }

    /// Returns a cloned accessor pinned to the requested block context so callers
    /// can run transport-side preflight logic without mutating shared API state.
    pub fn preflight_state_for_block_id(
        &self,
        block_id: Option<BlockId>,
        state: &mut ApiStateAccessor<S>,
    ) -> Result<ApiStateAccessor<S>, EthApiError> {
        let state = match self.resolve_state_for_block_id(block_id, state)? {
            super::maybe_archival_state::MaybeArchivalState::Current(current) => {
                current.clone_without_local_writes()
            }
            super::maybe_archival_state::MaybeArchivalState::Archival(state)
            | super::maybe_archival_state::MaybeArchivalState::Synthetic(state) => *state,
        };

        Ok(state)
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
        let mut normalized_request = request.clone();
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
        let mut auth_state = metered_state
            .clone_without_local_writes()
            .to_provable_reader();
        let (_, auth_data) =
            build_request_preflight_auth::<_, S>(&normalized_request, &mut auth_state).map_err(
                |err| into_rpc_error(format!("estimate_gas auth preflight failed: {err}")),
            )?;
        let signer = normalized_request.from.unwrap_or_default();
        let synthetic_nonce = Self::runtime_parity_nonce(&auth_data).map_err(into_rpc_error)?;
        let synthetic_signed_tx =
            Self::build_runtime_parity_signed_tx(&normalized_request, synthetic_nonce)
                .map_err(into_rpc_error)?;

        let ResultAndState {
            result,
            state: changes,
        } = self.call_with_context(
            request,
            block_env.clone(),
            maybe_archival_state,
            &cfg,
            state_overrides,
            block_overrides,
        )?;

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

        // Commit into the block-pinned RPC-local DB so post-simulation metering runs against
        // the same state snapshot used for the simulation itself.
        self.db(&mut metered_state)
            .try_commit(changes)
            .expect("Gas meter is initialized with INF");

        let pending_len = self
            .pending_transactions
            .len(&mut metered_state)
            .unwrap_infallible();
        let synthetic_tx =
            TxSignedAndRecovered::new(signer, synthetic_signed_tx, block_env.number.to::<u64>());
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
}

impl<S: Spec> Evm<S>
where
    S::Address: FromVmAddress<EthereumAddress>,
{
    /// Prepares the pinned pre-exec state and synthetic transaction needed to
    /// run the STF pipeline for runtime-parity gas estimation.
    #[cfg(feature = "native")]
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
    #[cfg(feature = "native")]
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

        Ok(U64::from(pending_tx.receipt.gas_used))
    }

    #[cfg(feature = "native")]
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

    #[cfg(feature = "native")]
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

    #[cfg(feature = "native")]
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

    #[cfg(feature = "native")]
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

    #[cfg(feature = "native")]
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
        request
            .from
            .ok_or_else(|| "normalized request missing from".to_string())?;
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

fn get_transaction_for_block_index<S: Spec>(
    evm: &Evm<S>,
    block: crate::MaybeSealedBlock,
    index: u64,
    state: &mut ApiStateAccessor<S>,
) -> Result<Option<Transaction>, EthApiError>
where
    S::Address: FromVmAddress<EthereumAddress>,
{
    let tx_count = block
        .transactions_end()
        .saturating_sub(block.transactions_start());
    if index >= tx_count {
        return Ok(None);
    }

    let tx_idx = block.transactions_start() + index;
    let tx = evm.tx(tx_idx, state)?;
    let tx = evm.build_tx_with_maybe_effective_gas_price(
        tx,
        block.hash(),
        block.number(),
        block.maybe_partial_header().base_fee_per_gas,
        index as usize, // safe: index < tx_count; block tx counts fit in usize on 64-bit targets
        tx_idx,
        state,
    );
    Ok(Some(tx))
}
