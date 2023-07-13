use anvil_core::eth::transaction::EthTransactionRequest;
use ethereum_types::{Address, H256, U256, U64};
use ethers_core::types::{Block, BlockId, FeeHistory, Transaction, TransactionReceipt, TxHash};
use sov_modules_macros::rpc_gen;
use sov_state::WorkingSet;
use tracing::info;

use crate::Evm;

#[rpc_gen(client, server, namespace = "eth")]
impl<C: sov_modules_api::Context> Evm<C> {
    #[rpc_method(name = "chainId")]
    pub fn chain_id(&self, _working_set: &mut WorkingSet<C::Storage>) -> Option<U64> {
        info!("evm module: eth_chainId");
        Some(U64::from(1u64))
    }

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

    #[rpc_method(name = "sendTransaction")]
    pub fn send_transaction(
        &self,
        _request: EthTransactionRequest,
        _working_set: &mut WorkingSet<C::Storage>,
    ) -> U256 {
        unimplemented!("eth_sendTransaction not implemented")
    }

    #[rpc_method(name = "blockNumber")]
    pub fn block_number(&self, _working_set: &mut WorkingSet<C::Storage>) -> U256 {
        unimplemented!("eth_blockNumber not implemented")
    }

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

    #[rpc_method(name = "getTransactionReceipt")]
    pub fn get_transaction_receipt(
        &self,
        _hash: H256,
        _working_set: &mut WorkingSet<C::Storage>,
    ) -> Option<TransactionReceipt> {
        unimplemented!("eth_getTransactionReceipt not implemented")
    }

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
