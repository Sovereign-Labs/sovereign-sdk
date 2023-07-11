use std::str::FromStr;

use ethereum_types::{Address, H256, U256, U64};
use ethers_core::types::transaction::eip2930::AccessListItem;
use ethers_core::types::{
    Block, BlockId, Bytes, FeeHistory, OtherFields, Transaction, TransactionReceipt, TxHash,
};
use sov_modules_macros::rpc_gen;
use sov_state::WorkingSet;
use tracing::info;

use crate::evm::EvmTransaction;
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
        hash: H256,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Option<Transaction> {
        info!("======= getTransactionByHash ===== {:?}", hash);
        let tx = self.transactions.get(&hash.into(), working_set);

        let tx = if let Some(tx) = tx {
            info!("Heeyy!!!");
            tx.into()
        } else {
            info!("Lool!!!");
            Transaction {
            block_hash: None,
            block_number: None,
            from: Address::from_str("c26ad91f4e7a0cad84c4b9315f420ca9217e315d").unwrap(),
            gas: U256::from_str_radix("0x10e2b", 16).unwrap(),
            gas_price: Some(U256::from_str_radix("0x12ec276caf", 16).unwrap()),
            hash: H256::from_str("929ff27a5c7833953df23103c4eb55ebdfb698678139d751c51932163877fada").unwrap(),
            input: Bytes::from(
                hex::decode("a9059cbb000000000000000000000000fdae129ecc2c27d166a3131098bc05d143fa258e0000000000000000000000000000000000000000000000000000000002faf080").unwrap()
            ),
            nonce: U256::zero(),
            to: Some(Address::from_str("dac17f958d2ee523a2206206994597c13d831ec7").unwrap()),
            transaction_index: None,
            value: U256::zero(),
            transaction_type: Some(U64::zero()),
            v: U64::from(0x25),
            r: U256::from_str_radix("c81e70f9e49e0d3b854720143e86d172fecc9e76ef8a8666f2fdc017017c5141", 16).unwrap(),
            s: U256::from_str_radix("1dd3410180f6a6ca3e25ad3058789cd0df3321ed76b5b4dbe0a2bb2dc28ae274", 16).unwrap(),
            chain_id: Some(U256::from(1)),
            access_list: None,
            max_fee_per_gas: None,
            max_priority_fee_per_gas: None,
            other: Default::default()
        }
        };

        Some(tx)
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

impl From<EvmTransaction> for Transaction {
    fn from(evm_tx: EvmTransaction) -> Self {
        Self {
            hash: evm_tx.hash.into(),
            nonce: evm_tx.nonce.into(),
            block_hash: Some([0; 32].into()),
            block_number: Some(1.into()),
            transaction_index: Some(1.into()),
            from: evm_tx.caller.into(),
            to: None,
            value: evm_tx.value.into(),
            gas_price: evm_tx.gas_price.map(|p| p.into()),
            gas: Default::default(),
            input: evm_tx.data.into(),
            v: Default::default(),
            r: Default::default(),
            s: Default::default(),
            transaction_type: Some(2.into()),
            access_list: None,
            max_priority_fee_per_gas: evm_tx.max_priority_fee_per_gas.map(|f| f.into()),
            max_fee_per_gas: Default::default(),
            chain_id: Some(1.into()),
            other: OtherFields::default(),
        }
    }
}
