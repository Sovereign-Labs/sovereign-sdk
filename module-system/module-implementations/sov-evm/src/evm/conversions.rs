use anvil_core::eth::transaction::{EIP1559Transaction, EthTransactionRequest};
use bytes::Bytes;
use ethers_core::types::{OtherFields, Transaction};
use revm::primitives::{
    AccountInfo as ReVmAccountInfo, BlockEnv as ReVmBlockEnv, Bytecode, CreateScheme, TransactTo,
    TxEnv, B160, B256, U256,
};

use super::transaction::{AccessListItem, BlockEnv, EvmTransaction};
use super::AccountInfo;

impl From<AccountInfo> for ReVmAccountInfo {
    fn from(info: AccountInfo) -> Self {
        Self {
            nonce: info.nonce,
            balance: U256::from_le_bytes(info.balance),
            code: Some(Bytecode::new_raw(Bytes::from(info.code))),
            code_hash: B256::from(info.code_hash),
        }
    }
}

impl From<ReVmAccountInfo> for AccountInfo {
    fn from(info: ReVmAccountInfo) -> Self {
        Self {
            balance: info.balance.to_le_bytes(),
            code_hash: info.code_hash.to_fixed_bytes(),
            code: info.code.unwrap_or_default().bytes().to_vec(),
            nonce: info.nonce,
        }
    }
}

impl From<BlockEnv> for ReVmBlockEnv {
    fn from(block_env: BlockEnv) -> Self {
        Self {
            number: U256::from_le_bytes(block_env.number),
            coinbase: B160::from_slice(&block_env.coinbase),
            timestamp: U256::from_le_bytes(block_env.timestamp),
            // TODO: handle difficulty
            difficulty: U256::ZERO,
            prevrandao: block_env.prevrandao.map(|r| B256::from_slice(&r)),
            basefee: U256::from_le_bytes(block_env.basefee),
            gas_limit: U256::from_le_bytes(block_env.gas_limit),
        }
    }
}

impl From<AccessListItem> for (B160, Vec<U256>) {
    fn from(item: AccessListItem) -> Self {
        (
            B160::from_slice(&item.address),
            item.storage_keys
                .into_iter()
                .map(U256::from_le_bytes)
                .collect(),
        )
    }
}

impl From<EvmTransaction> for TxEnv {
    fn from(tx: EvmTransaction) -> Self {
        let to = match tx.to {
            Some(addr) => TransactTo::Call(B160::from_slice(&addr)),
            None => TransactTo::Create(CreateScheme::Create),
        };

        let access_list = tx
            .access_lists
            .into_iter()
            .map(|item| item.into())
            .collect();

        Self {
            caller: B160::from_slice(&tx.sender),
            data: Bytes::from(tx.data),
            gas_limit: tx.gas_limit,
            gas_price: U256::from_be_bytes(tx.gas_price),
            gas_priority_fee: Some(U256::from_be_bytes(tx.max_priority_fee_per_gas)),
            transact_to: to,
            value: U256::from_be_bytes(tx.value),
            nonce: Some(tx.nonce),
            chain_id: Some(tx.chain_id),
            access_list,
        }
    }
}

impl From<EvmTransaction> for Transaction {
    fn from(evm_tx: EvmTransaction) -> Self {
        Self {
            hash: evm_tx.hash.into(),
            nonce: evm_tx.nonce.into(),
            from: evm_tx.sender.into(),
            to: evm_tx.to.map(|addr| addr.into()),
            value: evm_tx.value.into(),
            // https://github.com/foundry-rs/foundry/blob/master/anvil/core/src/eth/transaction/mod.rs#L1251
            gas_price: Some(evm_tx.max_fee_per_gas.into()),
            input: evm_tx.data.into(),
            v: (evm_tx.odd_y_parity as u8).into(),
            r: evm_tx.r.into(),
            s: evm_tx.s.into(),
            transaction_type: Some(2.into()),
            access_list: None,
            max_priority_fee_per_gas: Some(evm_tx.max_priority_fee_per_gas.into()),
            max_fee_per_gas: Some(evm_tx.max_fee_per_gas.into()),
            chain_id: Some(evm_tx.chain_id.into()),
            // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/503
            block_hash: Some([0; 32].into()),
            // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/503
            block_number: Some(1.into()),
            // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/503
            transaction_index: Some(1.into()),
            // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/503
            gas: Default::default(),
            // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/503
            other: OtherFields::default(),
        }
    }
}

impl From<EIP1559Transaction> for EvmTransaction {
    fn from(transaction: EIP1559Transaction) -> Self {
        let to = transaction.kind.as_call().map(|addr| (*addr).into());
        let tx_hash = transaction.hash();
        // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/515
        let sender = transaction.recover().unwrap();

        Self {
            sender: sender.into(),
            data: transaction.input.to_vec(),
            gas_limit: transaction.gas_limit.as_u64(),
            // https://github.com/foundry-rs/foundry/blob/master/anvil/core/src/eth/transaction/mod.rs#L1251C20-L1251C20
            gas_price: transaction.max_fee_per_gas.into(),
            max_priority_fee_per_gas: transaction.max_priority_fee_per_gas.into(),
            max_fee_per_gas: transaction.max_fee_per_gas.into(),
            to,
            value: transaction.value.into(),
            nonce: transaction.nonce.as_u64(),
            // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/503
            access_lists: vec![],
            chain_id: transaction.chain_id,
            // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/503
            hash: tx_hash.into(),
            odd_y_parity: transaction.odd_y_parity,
            r: transaction.r.into(),
            s: transaction.s.into(),
        }
    }
}

impl From<EthTransactionRequest> for EvmTransaction {
    fn from(req: EthTransactionRequest) -> Self {
        Self {
            sender: req.from.map(|addr| addr.into()).unwrap(),
            data: req.data.map(|d| d.to_vec()).unwrap_or_default(),
            gas_limit: req.gas.unwrap_or_default().as_u64(),
            gas_price: req.gas_price.unwrap_or_default().into(),
            max_priority_fee_per_gas: req.max_priority_fee_per_gas.unwrap_or_default().into(),
            max_fee_per_gas: req.max_fee_per_gas.unwrap_or_default().into(),
            to: req.to.map(|to| to.into()),
            value: req.value.unwrap_or_default().into(),
            nonce: req.nonce.unwrap_or_default().as_u64(),
            // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/503
            access_lists: Default::default(),
            chain_id: req.chain_id.unwrap_or_default().as_u64(),
            // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/515
            odd_y_parity: Default::default(),
            // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/515
            r: Default::default(),
            // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/515
            s: Default::default(),
            // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/503
            hash: Default::default(),
        }
    }
}
