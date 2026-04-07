use crate::error::into_rpc_error;
use crate::rpc::error::ensure_success;
use alloy_consensus::ReceiptEnvelope;
use alloy_eips::BlockId;
use alloy_primitives::{Address, U64};
use alloy_primitives::{Bytes, B256, U256};
use alloy_rpc_types::{
    state::StateOverride, AccessListResult, Block, BlockNumberOrTag, BlockOverrides, FeeHistory,
    Transaction, TransactionReceipt, TransactionRequest,
};
use alloy_rpc_types_trace::geth::GethDebugTracingOptions;
use alloy_rpc_types_trace::geth::{GethTrace, TraceResult};
use jsonrpsee::core::RpcResult;
use revm::context::result::ExecutionResult;
use revm::Database;
use revm_inspectors::access_list::AccessListInspector;
use sov_address::{EthereumAddress, FromVmAddress};
use sov_modules_api::macros::{config_value, rpc_gen};
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::{ApiStateAccessor, Spec};
use sov_rpc_eth_types::{EthApiError, LogWithExecutionTimestamp};
use std::ops::DerefMut;
use tracing::trace;

use crate::Evm;

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
    ///
    /// Simulates a transaction without committing state changes.
    ///
    /// # Affordability checks
    ///
    /// No balance check is performed when fee fields are omitted (the common case).
    /// [`prepare_call_env`](crate::helpers::prepare_call_env) hardcodes `gas_price = 0`,
    /// making revm's upfront-cost formula evaluate to zero so any account can call.
    /// When `value > 0`, revm still verifies the caller holds at least `value`.
    ///
    /// **Desired behaviour** (matching geth): when the caller explicitly provides
    /// `gasPrice > 0` or `maxFeePerGas > 0`, enforce `gas_limit * gas_price + value
    /// <= balance` and return `InsufficientFunds` on failure. This is not yet
    /// implemented — see the divergence note on [`prepare_call_env`](crate::helpers::prepare_call_env).
    ///
    /// This differs from `eth_estimateGas`, which runs a paymaster-aware
    /// affordability preflight in `sov-ethereum` (see `estimate_gas.rs`).
    ///
    /// References:
    /// - Geth `doCall`: <https://github.com/ethereum/go-ethereum/blob/master/internal/ethapi/api.go>
    /// - Reth `call`: <https://github.com/paradigmxyz/reth/blob/main/crates/rpc/rpc-eth-api/src/helpers/call.rs>
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
    ///
    /// Generates an EIP-2930 access list by running the transaction with an
    /// [`AccessListInspector`]. Affordability semantics are identical to `eth_call`:
    /// no balance check when fee fields are omitted, because
    /// [`prepare_call_env`](crate::helpers::prepare_call_env) sets `gas_price = 0`.
    /// This matches geth's `AccessList()` path which also uses zero-fee defaults.
    ///
    /// Note: EIP-2930 (<https://eips.ethereum.org/EIPS/eip-2930>) defines the access
    /// list *transaction type*; the `eth_createAccessList` RPC method itself is a
    /// client-level addition defined in `execution-apis`
    /// (<https://github.com/ethereum/execution-apis>).
    ///
    /// See [`prepare_call_env`](crate::helpers::prepare_call_env) for the full
    /// affordability analysis and current divergence from geth.
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
