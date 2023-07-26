use ethereum_types::{Address, H256, U256, U64};
use ethers_core::types::transaction::eip2930::AccessListItem;
use ethers_core::types::{
    Block, BlockId, Bytes, FeeHistory, Transaction, TransactionReceipt, TxHash,
};
use sov_modules_macros::rpc_gen;
use sov_state::WorkingSet;
use tracing::info;

use crate::Evm;

#[derive(Clone, Debug, PartialEq, Eq, Default, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub struct EthTransactionRequest {
    /// from address
    pub from: Option<Address>,
    /// to address
    pub to: Option<Address>,
    /// legacy, gas Price
    #[cfg_attr(feature = "serde", serde(default))]
    pub gas_price: Option<U256>,
    /// max base fee per gas sender is willing to pay
    #[cfg_attr(feature = "serde", serde(default))]
    pub max_fee_per_gas: Option<U256>,
    /// miner tip
    #[cfg_attr(feature = "serde", serde(default))]
    pub max_priority_fee_per_gas: Option<U256>,
    /// gas
    pub gas: Option<U256>,
    /// value of th tx in wei
    pub value: Option<U256>,
    /// Any additional data sent
    pub data: Option<Bytes>,
    /// Transaction nonce
    pub nonce: Option<U256>,
    /// chain id
    #[cfg_attr(feature = "serde", serde(default))]
    pub chain_id: Option<U64>,
    /// warm storage access pre-payment
    #[cfg_attr(feature = "serde", serde(default))]
    pub access_list: Option<Vec<AccessListItem>>,
    /// EIP-2718 type
    #[cfg_attr(feature = "serde", serde(rename = "type"))]
    pub transaction_type: Option<U256>,
}

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
        _hash: H256,
        _working_set: &mut WorkingSet<C::Storage>,
    ) -> Option<Transaction> {
        unimplemented!("eth_blockNumber not implemented")
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
