use bytes::Bytes;
use ethers_core::types::{OtherFields, Transaction};
use reth_rpc::eth::error::{EthApiError, RpcInvalidTransactionError};
use reth_rpc_types::CallRequest;
use revm::primitives::{
    AccountInfo as ReVmAccountInfo, BlockEnv as ReVmBlockEnv, Bytecode, CreateScheme, TransactTo,
    TxEnv, B160, B256, U256,
};

use super::transaction::{AccessListItem, BlockEnv, EvmTransaction, Signature};
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
            gas_price: U256::from(tx.gas_price),
            gas_priority_fee: Some(U256::from(tx.max_priority_fee_per_gas)),
            transact_to: to,
            value: U256::from(tx.value),
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
            v: (evm_tx.sig.odd_y_parity as u8).into(),
            r: evm_tx.sig.r.into(),
            s: evm_tx.sig.s.into(),
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

use reth_primitives::{
    Bytes as RethBytes, Signature as RethSignature, TransactionSigned as RethTransactionSigned,
};

impl TryFrom<RethBytes> for EvmTransaction {
    type Error = EthApiError;

    fn try_from(data: RethBytes) -> Result<Self, Self::Error> {
        if data.is_empty() {
            return Err(EthApiError::EmptyRawTransactionData);
        }

        let transaction = RethTransactionSigned::decode_enveloped(data)
            .map_err(|_| EthApiError::FailedToDecodeSignedTransaction)?;

        let transaction = transaction
            .into_ecrecovered()
            .ok_or(EthApiError::InvalidTransactionSignature)?;

        let (signed_transaction, signer) = transaction.to_components();

        let tx_hash = signed_transaction.hash();
        let tx_eip_1559 = match signed_transaction.transaction {
            reth_primitives::Transaction::Legacy(_) => {
                return Err(EthApiError::InvalidTransaction(
                    RpcInvalidTransactionError::TxTypeNotSupported,
                ))
            }
            reth_primitives::Transaction::Eip2930(_) => {
                return Err(EthApiError::InvalidTransaction(
                    RpcInvalidTransactionError::TxTypeNotSupported,
                ))
            }
            reth_primitives::Transaction::Eip1559(tx_eip_1559) => tx_eip_1559,
        };

        Ok(Self {
            sender: signer.into(),
            data: tx_eip_1559.input.to_vec(),
            gas_limit: tx_eip_1559.gas_limit,
            // https://github.com/foundry-rs/foundry/blob/master/anvil/core/src/eth/transaction/mod.rs#L1251C20-L1251C20
            gas_price: tx_eip_1559.max_fee_per_gas,
            max_priority_fee_per_gas: tx_eip_1559.max_priority_fee_per_gas,
            max_fee_per_gas: tx_eip_1559.max_fee_per_gas,
            to: tx_eip_1559.to.to().map(|addr| addr.into()),
            value: tx_eip_1559.value,
            nonce: tx_eip_1559.nonce,
            // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/503
            access_lists: vec![],
            chain_id: tx_eip_1559.chain_id,
            // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/503
            hash: tx_hash.into(),
            sig: signed_transaction.signature.into(),
        })
    }
}

impl From<RethSignature> for Signature {
    fn from(sig: RethSignature) -> Self {
        Self {
            s: sig.s.to_be_bytes(),
            r: sig.r.to_be_bytes(),
            odd_y_parity: sig.odd_y_parity,
        }
    }
}

// TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/576
// https://github.com/paradigmxyz/reth/blob/d8677b4146f77c7c82d659c59b79b38caca78778/crates/rpc/rpc/src/eth/revm_utils.rs#L201
pub fn prepare_call_env(request: CallRequest) -> TxEnv {
    TxEnv {
        caller: request.from.unwrap(),
        gas_limit: request.gas.map(|p| p.try_into().unwrap()).unwrap(),
        gas_price: request.gas_price.unwrap_or_default(),
        gas_priority_fee: request.max_priority_fee_per_gas,
        transact_to: request
            .to
            .map(TransactTo::Call)
            .unwrap_or_else(TransactTo::create),
        value: request.value.unwrap_or_default(),
        data: request
            .input
            .try_into_unique_input()
            .unwrap()
            .map(|data| data.0)
            .unwrap_or_default(),
        chain_id: request.chain_id.map(|c| c.as_u64()),
        nonce: request.nonce.map(|n| TryInto::<u64>::try_into(n).unwrap()),
        access_list: Default::default(),
    }
}
