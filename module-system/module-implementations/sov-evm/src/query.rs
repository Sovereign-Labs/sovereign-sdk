use anvil_core::eth::state::StateOverride;
use anvil_core::eth::transaction::EthTransactionRequest;
use ethereum_types::{Address, H256, U256, U64};
use ethers::types::Bytes;
use ethers_core::types::{Block, BlockId, FeeHistory, Transaction, TransactionReceipt, TxHash};
use revm::primitives::{CfgEnv, ExecutionResult, U256 as EVM_U256};
use sov_modules_macros::rpc_gen;
use sov_state::WorkingSet;
use tracing::info;

use crate::evm::db::EvmDb;
use crate::evm::{executor, EvmTransaction};
use crate::Evm;

#[rpc_gen(client, server, namespace = "eth")]
impl<C: sov_modules_api::Context> Evm<C> {
    // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/502
    #[rpc_method(name = "chainId")]
    pub fn chain_id(&self, _working_set: &mut WorkingSet<C::Storage>) -> Option<U64> {
        info!("evm module: eth_chainId");
        Some(U64::from(1u64))
    }

    // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/502
    #[rpc_method(name = "getBlockByNumber")]
    pub fn get_block_by_number(
        &self,
        _block_number: Option<String>,
        _details: Option<bool>,
        _working_set: &mut WorkingSet<C::Storage>,
    ) -> Option<Block<TxHash>> {
        info!("evm module: eth_getBlockByNumber");

        let block = Block::<TxHash> {
            base_fee_per_gas: Some(100.into()),
            ..Default::default()
        };

        Some(block)
    }

    // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/502
    #[rpc_method(name = "feeHistory")]
    pub fn fee_history(&self, _working_set: &mut WorkingSet<C::Storage>) -> FeeHistory {
        info!("evm module: eth_feeHistory");
        FeeHistory {
            base_fee_per_gas: Default::default(),
            gas_used_ratio: Default::default(),
            oldest_block: Default::default(),
            reward: Default::default(),
        }
    }

    // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/502
    #[rpc_method(name = "getTransactionByHash")]
    pub fn get_transaction_by_hash(
        &self,
        hash: H256,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Option<Transaction> {
        info!("evm module: eth_getTransactionByHash");
        let evm_transaction = self.transactions.get(&hash.into(), working_set);
        evm_transaction.map(|tx| tx.into())
    }

    // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/502
    #[rpc_method(name = "getTransactionReceipt")]
    pub fn get_transaction_receipt(
        &self,
        hash: H256,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Option<TransactionReceipt> {
        info!("evm module: eth_getTransactionReceipt");

        let receipt = self.receipts.get(&hash.into(), working_set);
        receipt.map(|r| r.into())
    }

    // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/502
    #[rpc_method(name = "call")]
    pub fn get_call(
        &self,
        // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/501
        request: EthTransactionRequest,
        _block_number: Option<BlockId>,
        // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/501
        _overrides: Option<StateOverride>,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Bytes {
        info!("evm module: eth_call");
        let tx: EvmTransaction = request.into();

        // https://github.com/Sovereign-Labs/sovereign-sdk/issues/516
        let cfg_env = CfgEnv {
            chain_id: EVM_U256::ZERO,
            ..Default::default()
        };

        let block_env = self.block_env.get(working_set).unwrap_or_default();
        let evm_db: EvmDb<'_, C> = self.get_db(working_set);

        // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/505
        let result = executor::inspect(evm_db, block_env, tx, cfg_env).unwrap();
        let output = match result.result {
            ExecutionResult::Success { output, .. } => output,
            _ => todo!(),
        };
        output.into_data().into()
    }

    // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/502
    #[rpc_method(name = "sendTransaction")]
    pub fn send_transaction(
        &self,
        // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/501
        _request: EthTransactionRequest,
        _working_set: &mut WorkingSet<C::Storage>,
    ) -> U256 {
        unimplemented!("eth_sendTransaction not implemented")
    }

    // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/502
    #[rpc_method(name = "blockNumber")]
    pub fn block_number(&self, _working_set: &mut WorkingSet<C::Storage>) -> U256 {
        unimplemented!("eth_blockNumber not implemented")
    }

    // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/502
    #[rpc_method(name = "getTransactionCount")]
    pub fn get_transaction_count(
        &self,
        _address: Address,
        _block_number: Option<BlockId>,
        _working_set: &mut WorkingSet<C::Storage>,
    ) -> Option<U256> {
        unimplemented!("eth_getTransactionCount not implemented")
    }
}
