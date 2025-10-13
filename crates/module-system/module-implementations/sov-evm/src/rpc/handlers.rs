use crate::db::commit::FallibleDatabaseCommit;
use crate::rpc::error::ensure_success;
use alloy_primitives::{Address, U64};
use alloy_primitives::{Bytes, B256, U256};
use alloy_rpc_types::{
    state::StateOverride, Block, BlockNumberOrTag, BlockOverrides, FeeHistory, Transaction,
    TransactionReceipt, TransactionRequest,
};
use alloy_rpc_types_trace::geth::GethDebugTracingOptions;
use alloy_rpc_types_trace::geth::{GethTrace, TraceResult};
use jsonrpsee::core::RpcResult;
use revm::context::result::ResultAndState;
use revm::Database;
use sov_address::{EthereumAddress, FromVmAddress};
use sov_modules_api::macros::{config_value, rpc_gen};
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::{ApiStateAccessor, GasMeter, GasSpec, Spec};
use sov_rpc_eth_types::EthApiError;
use tracing::debug;

use crate::{apply_margins, Evm};
use std::ops::DerefMut;

const EMPTY_FEE_HISTORY: FeeHistory = FeeHistory {
    base_fee_per_gas: vec![],
    gas_used_ratio: vec![],
    oldest_block: 0,
    reward: None,
    blob_gas_used_ratio: vec![],
    // EIP-4844 related
    base_fee_per_blob_gas: vec![],
};

#[rpc_gen(client, server)]
impl<S: Spec> Evm<S>
where
    S::Address: FromVmAddress<EthereumAddress>,
{
    /// Handler for `net_version`
    #[rpc_method(name = "net_version")]
    pub fn net_version(&self, _state: &mut ApiStateAccessor<S>) -> RpcResult<String> {
        debug!("EVM module JSON-RPC request to `net_version`");

        // Network ID is the same as chain ID for most networks
        let chain_id = config_value!("CHAIN_ID");
        Ok(chain_id.to_string())
    }

    /// Handler for: `eth_chainId`
    #[rpc_method(name = "eth_chainId")]
    pub fn chain_id(&self, _state: &mut ApiStateAccessor<S>) -> RpcResult<Option<U64>> {
        let chain_id = config_value!("CHAIN_ID");
        debug!(
            chain_id = chain_id,
            "EVM module JSON-RPC request to `eth_chainId`"
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
        debug!(
            ?block_hash,
            "EVM module JSON-RPC request to `eth_getBlockByHash`"
        );

        let block_number_hex = self
            .block_hashes
            .get(&block_hash, state)
            .unwrap_infallible()
            .map(|number| hex::encode(number.to_be_bytes()));
        let kind = details.unwrap_or_default().into();
        Ok(match block_number_hex {
            Some(block_number_hex) => self.get_block(Some(block_number_hex), kind, state),
            None => None,
        })
    }

    /// Handler for: `eth_getBlockByNumber`
    #[rpc_method(name = "eth_getBlockByNumber")]
    pub fn get_block_by_number(
        &self,
        block_number: Option<String>,
        details: Option<bool>,
        state: &mut ApiStateAccessor<S>,
    ) -> RpcResult<Option<Block>> {
        debug!(
            block_number,
            "EVM module JSON-RPC request to `eth_getBlockByNumber`"
        );
        let kind = details.unwrap_or_default().into();
        Ok(self.get_block(block_number, kind, state))
    }

    /// Handler for: `eth_getBalance`
    #[rpc_method(name = "eth_getBalance")]
    pub fn get_balance(
        &self,
        address: Address,
        block_number: Option<String>,
        state: &mut ApiStateAccessor<S>,
    ) -> RpcResult<U256> {
        let mut state = self.resolve_state(block_number, state)?;
        let balance = self
            .get_db(state.deref_mut())
            .basic(address)
            .map_err(EthApiError::from)?
            .map(|account| account.balance)
            .unwrap_or_default();

        debug!(
            %address,
            %balance,
            "EVM module JSON-RPC request to `eth_getBalance`"
        );

        Ok(balance)
    }

    /// Handler for: `eth_getStorageAt`
    #[rpc_method(name = "eth_getStorageAt")]
    pub fn get_storage_at(
        &self,
        address: Address,
        index: U256,
        block_number: Option<String>,
        state: &mut ApiStateAccessor<S>,
    ) -> RpcResult<U256> {
        debug!("EVM module JSON-RPC request to `eth_getStorageAt`");

        let mut state = self.resolve_state(block_number, state)?;
        let storage_slot = self
            .account_storage
            .get(&(&address, &index), state.deref_mut())
            .unwrap_infallible()
            .unwrap_or_default();

        Ok(storage_slot)
    }

    /// Handler for: `eth_getTransactionCount`
    #[rpc_method(name = "eth_getTransactionCount")]
    pub fn get_transaction_count(
        &self,
        address: Address,
        block_number: Option<String>,
        state: &mut ApiStateAccessor<S>,
    ) -> RpcResult<U64> {
        let mut state = self.resolve_state(block_number, state)?;

        let ethereum_address: EthereumAddress = address.into();
        let credential_id = ethereum_address.as_credential_id();

        let nonce = self
            .uniqueness_module
            .next_nonce(&credential_id, state.deref_mut())
            .unwrap_or_default();

        debug!(%address, nonce, "EVM module JSON-RPC request to `eth_getTransactionCount`");
        Ok(U64::from(nonce))
    }

    /// Handler for: `eth_getCode`
    #[rpc_method(name = "eth_getCode")]
    pub fn get_code(
        &self,
        address: Address,
        block_number: Option<String>,
        state: &mut ApiStateAccessor<S>,
    ) -> RpcResult<Bytes> {
        debug!("EVM module JSON-RPC request to `eth_getCode`");
        let state = self.resolve_state(block_number, state)?;
        Ok(self.get_contract_code(address, state).unwrap_or_default())
    }

    /// Handler for: `eth_feeHistory`
    // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/502
    #[rpc_method(name = "eth_feeHistory")]
    pub fn fee_history(&self) -> RpcResult<FeeHistory> {
        debug!("EVM module JSON-RPC request to `eth_feeHistory`");
        Ok(EMPTY_FEE_HISTORY)
    }

    /// Handler for: `eth_getTransactionByHash`
    #[rpc_method(name = "eth_getTransactionByHash")]
    pub fn get_transaction_by_hash(
        &self,
        hash: B256,
        state: &mut ApiStateAccessor<S>,
    ) -> RpcResult<Option<Transaction>> {
        let transaction = self.get_transaction(hash, state);
        debug!(
            %hash,
            ?transaction,
            "EVM module JSON-RPC request to `eth_getTransactionByHash`"
        );
        Ok(transaction)
    }

    /// Handler for: `eth_getBlockReceipts`
    #[rpc_method(name = "eth_getBlockReceipts")]
    pub fn get_block_receipts(
        &self,
        block_number: Option<String>,
        state: &mut ApiStateAccessor<S>,
    ) -> RpcResult<Option<Vec<TransactionReceipt>>> {
        debug!(
            block_number,
            "EVM module JSON-RPC request to `eth_getBlockReceipts`"
        );
        Ok(self.get_receipts(block_number, state))
    }

    /// Handler for: `eth_getTransactionReceipt`
    #[rpc_method(name = "eth_getTransactionReceipt")]
    pub fn get_transaction_receipt(
        &self,
        hash: B256,
        state: &mut ApiStateAccessor<S>,
    ) -> RpcResult<Option<TransactionReceipt>> {
        debug!(
            %hash,
            "EVM module JSON-RPC request to `eth_getTransactionReceipt`"
        );
        Ok(self.get_receipt_by_hash(hash, state))
    }

    /// Handler for: `eth_call`
    //https://github.com/paradigmxyz/reth/blob/f577e147807a783438a3f16aad968b4396274483/crates/rpc/rpc/src/eth/api/transactions.rs#L502
    #[rpc_method(name = "eth_call")]
    pub fn eth_call(
        &self,
        request: TransactionRequest,
        block_number: Option<String>,
        _state_overrides: Option<StateOverride>,
        _block_overrides: Option<Box<BlockOverrides>>,
        state: &mut ApiStateAccessor<S>,
    ) -> RpcResult<Bytes> {
        debug!("EVM module JSON-RPC request to `eth_call`");
        let result = self.call(request, block_number, state)?.result;
        Ok(ensure_success(result)?)
    }

    /// Handler for: `eth_blockNumber`
    #[rpc_method(name = "eth_blockNumber")]
    pub fn block_number(&self, state: &mut ApiStateAccessor<S>) -> RpcResult<U256> {
        debug!("EVM module JSON-RPC request to `eth_blockNumber`");
        let block_number_range = self
            .block_numbers
            .get(state)
            .unwrap_infallible()
            // Justified, we set it at genesis and later only override it.
            .expect("The impossible happened: block_numbers was not set.");

        Ok(U256::from(*block_number_range.end()))
    }

    /// Handler for: `eth_estimateGas`
    // https://github.com/paradigmxyz/reth/blob/main/crates/rpc/rpc/src/eth/api/call.rs#L172
    #[rpc_method(name = "eth_estimateGas")]
    pub fn eth_estimate_gas(
        &self,
        request: TransactionRequest,
        block_number: Option<String>,
        state: &mut ApiStateAccessor<S>,
    ) -> RpcResult<U64> {
        debug!("EVM module JSON-RPC request to `eth_estimateGas`");
        let ResultAndState {
            result,
            state: changes,
        } = self.call(request, block_number, state)?;
        self.get_db(state)
            .commit(changes)
            .expect("Impossible as gas meter is initialized with INF");
        let gas_used = result.gas_used();
        let gas_meter = state.try_as_basic_gas_meter().unwrap();
        gas_meter
            .charge_linear_gas(<S as GasSpec>::gas_to_charge_per_evm_gas(), gas_used as u32)
            .expect("No underflow is possible here as we init EVM gas with gas meter gas");
        let total_gas_used =
            gas_meter.initial_gas.as_ref()[0] - gas_meter.remaining_gas.as_ref()[0];
        Ok(U64::from(apply_margins(total_gas_used)?))
    }

    /// Handler for `debug_traceBlockByNumber`
    #[rpc_method(name = "debug_traceBlockByNumber")]
    pub fn debug_trace_block_by_number(
        &self,
        block: BlockNumberOrTag,
        opts: Option<GethDebugTracingOptions>,
        state: &mut ApiStateAccessor<S>,
    ) -> RpcResult<Vec<TraceResult>> {
        debug!("EVM module JSON-RPC request to `debug_traceBlockByNumber`");
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
        debug!("EVM module JSON-RPC request to `debug_traceTransaction`");
        Ok(self.trace_transaction(tx_hash, opts.unwrap_or_default(), state)?)
    }
}
