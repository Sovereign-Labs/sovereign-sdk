use jsonrpsee::core::RpcResult;
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
    pub fn chain_id(
        &self,
        _working_set: &mut WorkingSet<C::Storage>,
    ) -> RpcResult<Option<reth_primitives::U64>> {
        info!("evm module: eth_chainId");
        Ok(Some(reth_primitives::U64::from(1u64)))
    }

    // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/502
    #[rpc_method(name = "getBlockByNumber")]
    pub fn get_block_by_number(
        &self,
        _block_number: Option<String>,
        _details: Option<bool>,
        _working_set: &mut WorkingSet<C::Storage>,
    ) -> RpcResult<Option<reth_rpc_types::RichBlock>> {
        info!("evm module: eth_getBlockByNumber");

        let header = reth_rpc_types::Header {
            hash: Default::default(),
            parent_hash: Default::default(),
            uncles_hash: Default::default(),
            miner: Default::default(),
            state_root: Default::default(),
            transactions_root: Default::default(),
            receipts_root: Default::default(),
            logs_bloom: Default::default(),
            difficulty: Default::default(),
            number: Default::default(),
            gas_limit: Default::default(),
            gas_used: Default::default(),
            timestamp: Default::default(),
            extra_data: Default::default(),
            mix_hash: Default::default(),
            nonce: Default::default(),
            base_fee_per_gas: Some(reth_primitives::U256::from(100)),
            withdrawals_root: Default::default(),
        };

        let block = reth_rpc_types::Block {
            header,
            total_difficulty: Default::default(),
            uncles: Default::default(),
            transactions: reth_rpc_types::BlockTransactions::Hashes(Default::default()),
            size: Default::default(),
            withdrawals: Default::default(),
        };

        Ok(Some(block.into()))
    }

    // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/502
    #[rpc_method(name = "feeHistory")]
    pub fn fee_history(
        &self,
        _working_set: &mut WorkingSet<C::Storage>,
    ) -> RpcResult<reth_rpc_types::FeeHistory> {
        info!("evm module: eth_feeHistory");
        Ok(reth_rpc_types::FeeHistory {
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
        hash: reth_primitives::H256,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> RpcResult<Option<reth_rpc_types::Transaction>> {
        info!("evm module: eth_getTransactionByHash");
        //let evm_transaction = self.transactions.get(&hash, working_set);
        let evm_transaction = self
            .transactions
            .get(&hash, &mut working_set.accessory_state());
        Ok(evm_transaction)
    }

    // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/502
    #[rpc_method(name = "getTransactionReceipt")]
    pub fn get_transaction_receipt(
        &self,
        hash: reth_primitives::U256,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> RpcResult<Option<reth_rpc_types::TransactionReceipt>> {
        info!("evm module: eth_getTransactionReceipt");

        let receipt = self.receipts.get(&hash, &mut working_set.accessory_state());
        Ok(receipt)
    }

    //https://github.com/paradigmxyz/reth/blob/f577e147807a783438a3f16aad968b4396274483/crates/rpc/rpc/src/eth/api/transactions.rs#L502
    //https://github.com/paradigmxyz/reth/blob/main/crates/rpc/rpc-types/src/eth/call.rs#L7

    /// Template env for eth_call
    const CALL_CFG_ENV_TEMPLATE: revm::primitives::CfgEnv = revm::primitives::CfgEnv {
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
        request: reth_rpc_types::CallRequest,
        _block_number: Option<reth_primitives::BlockId>,
        // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/501
        _state_overrides: Option<reth_rpc_types::state::StateOverride>,
        _block_overrides: Option<Box<reth_rpc_types::BlockOverrides>>,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> RpcResult<reth_primitives::Bytes> {
        info!("evm module: eth_call");
        let tx_env = prepare_call_env(request);

        let block_env = self.pending_block.get(working_set).unwrap_or_default();
        let cfg = self.cfg.get(working_set).unwrap_or_default();
        let cfg_env = get_cfg_env(&block_env, cfg, Some(Self::CALL_CFG_ENV_TEMPLATE));

        let evm_db: EvmDb<'_, C> = self.get_db(working_set);

        // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/505
        let result = executor::inspect(evm_db, block_env, tx_env, cfg_env).unwrap();
        let output = match result.result {
            revm::primitives::ExecutionResult::Success { output, .. } => output,
            _ => todo!(),
        };
        Ok(output.into_data().into())
    }

    // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/502
    #[rpc_method(name = "sendTransaction")]
    pub fn send_transaction(
        &self,
        // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/501
        _request: reth_rpc_types::TransactionRequest,
        _working_set: &mut WorkingSet<C::Storage>,
    ) -> RpcResult<reth_primitives::U256> {
        unimplemented!("eth_sendTransaction not implemented")
    }

    // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/502
    #[rpc_method(name = "blockNumber")]
    pub fn block_number(
        &self,
        _working_set: &mut WorkingSet<C::Storage>,
    ) -> RpcResult<reth_primitives::U256> {
        unimplemented!("eth_blockNumber not implemented")
    }
}
