use ethereum_types::{Address, H256, U256, U64};
use ethers::types::Bytes;
use ethers_core::types::{Block, BlockId, FeeHistory, Transaction, TransactionReceipt, TxHash};
use jsonrpsee::core::RpcResult;
use reth_rpc_types::state::StateOverride;
use reth_rpc_types::{BlockOverrides, CallRequest, TransactionRequest};
use revm::primitives::{CfgEnv, ExecutionResult};
use sov_modules_api::macros::rpc_gen;
use sov_state::WorkingSet;
use tracing::info;

use crate::call::get_cfg_env;
use crate::evm::db::EvmDb;
use crate::evm::{executor, prepare_call_env};
use crate::Evm;

#[rpc_gen(client, server, namespace = "eth")]
impl<C: sov_modules_api::Context> Evm<C> {
    // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/502
    #[rpc_method(name = "chainId")]
    pub fn chain_id(&self, _working_set: &mut WorkingSet<C::Storage>) -> RpcResult<Option<U64>> {
        info!("evm module: eth_chainId");
        Ok(Some(U64::from(1u64)))
    }

    // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/502
    #[rpc_method(name = "getBlockByNumber")]
    pub fn get_block_by_number(
        &self,
        _block_number: Option<String>,
        _details: Option<bool>,
        _working_set: &mut WorkingSet<C::Storage>,
    ) -> RpcResult<Option<Block<TxHash>>> {
        info!("evm module: eth_getBlockByNumber");

        let block = Block::<TxHash> {
            base_fee_per_gas: Some(100.into()),
            ..Default::default()
        };

        Ok(Some(block))
    }

    // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/502
    #[rpc_method(name = "feeHistory")]
    pub fn fee_history(&self, _working_set: &mut WorkingSet<C::Storage>) -> RpcResult<FeeHistory> {
        info!("evm module: eth_feeHistory");
        Ok(FeeHistory {
            base_fee_per_gas: Default::default(),
            gas_used_ratio: Default::default(),
            oldest_block: Default::default(),
            reward: Default::default(),
        })
    }

    // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/502
    #[rpc_method(name = "getTransactionByHash")]
    pub fn get_transaction_by_hash(
        &self,
        hash: H256,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> RpcResult<Option<Transaction>> {
        info!("evm module: eth_getTransactionByHash");
        let evm_transaction = self.transactions.get(&hash.into(), working_set);
        Ok(evm_transaction.map(|tx| tx.into()))
    }

    // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/502
    #[rpc_method(name = "getTransactionReceipt")]
    pub fn get_transaction_receipt(
        &self,
        hash: H256,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> RpcResult<Option<TransactionReceipt>> {
        info!("evm module: eth_getTransactionReceipt");
        let receipt = self.receipts.get(&hash.into(), working_set);
        Ok(receipt.map(|r| r.into()))
    }

    //https://github.com/paradigmxyz/reth/blob/f577e147807a783438a3f16aad968b4396274483/crates/rpc/rpc/src/eth/api/transactions.rs#L502
    //https://github.com/paradigmxyz/reth/blob/main/crates/rpc/rpc-types/src/eth/call.rs#L7

    /// Template env for eth_call
    const CALL_CFG_ENV_TEMPLATE: CfgEnv = CfgEnv {
        // Reth sets this to true and uses only timeout, but other clients use this as a part of DOS attacks protection, with 100mln gas limit
        // https://github.com/paradigmxyz/reth/blob/62f39a5a151c5f4ddc9bf0851725923989df0412/crates/rpc/rpc/src/eth/revm_utils.rs#L215
        disable_block_gas_limit: false,
        disable_eip3607: true,
        disable_base_fee: true,
        chain_id: revm::primitives::U256::ZERO,
        spec_id: revm::primitives::SpecId::LATEST,
        perf_all_precompiles_have_balance: false,
        perf_analyse_created_bytecodes: revm::primitives::AnalysisKind::Analyse,
        limit_contract_code_size: None,
    };

    // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/502
    #[rpc_method(name = "call")]
    pub fn get_call(
        &self,
        // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/501
        request: CallRequest,
        _block_number: Option<BlockId>,
        // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/501
        _state_overrides: Option<StateOverride>,
        _block_overrides: Option<Box<BlockOverrides>>,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> RpcResult<Bytes> {
        info!("evm module: eth_call");
        let tx_env = prepare_call_env(request);

        let block_env = self.block_env.get(working_set).unwrap_or_default();
        let cfg = self.cfg.get(working_set).unwrap_or_default();
        let cfg_env = get_cfg_env(&block_env, cfg, Some(Self::CALL_CFG_ENV_TEMPLATE));

        let evm_db: EvmDb<'_, C> = self.get_db(working_set);

        // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/505
        let result = executor::inspect(evm_db, block_env, tx_env, cfg_env).unwrap();
        let output = match result.result {
            ExecutionResult::Success { output, .. } => output,
            _ => todo!(),
        };
        Ok(output.into_data().into())
    }

    // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/502
    #[rpc_method(name = "sendTransaction")]
    pub fn send_transaction(
        &self,
        // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/501
        _request: TransactionRequest,
        _working_set: &mut WorkingSet<C::Storage>,
    ) -> RpcResult<U256> {
        unimplemented!("eth_sendTransaction not implemented")
    }

    // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/502
    #[rpc_method(name = "blockNumber")]
    pub fn block_number(&self, _working_set: &mut WorkingSet<C::Storage>) -> RpcResult<U256> {
        unimplemented!("eth_blockNumber not implemented")
    }

    // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/502
    #[rpc_method(name = "getTransactionCount")]
    pub fn get_transaction_count(
        &self,
        _address: Address,
        _block_number: Option<BlockId>,
        _working_set: &mut WorkingSet<C::Storage>,
    ) -> RpcResult<Option<U256>> {
        unimplemented!("eth_getTransactionCount not implemented")
    }
}
