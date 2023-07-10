use bytes::Bytes;
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
            caller: B160::from_slice(&tx.caller),
            data: Bytes::from(tx.data),
            gas_limit: tx.gas_limit,
            gas_price: tx.gas_price.map(U256::from_le_bytes).unwrap_or_default(),
            gas_priority_fee: tx.max_priority_fee_per_gas.map(U256::from_le_bytes),
            transact_to: to,
            value: U256::from_le_bytes(tx.value),
            nonce: Some(tx.nonce),
            //TODO: handle chain_id
            chain_id: None,
            access_list,
        }
    }
}
