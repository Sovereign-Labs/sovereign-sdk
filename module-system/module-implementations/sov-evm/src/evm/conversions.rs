use bytes::Bytes;
use reth_primitives::{
    AccessList, Bytes as RethBytes, TransactionSigned, TransactionSignedEcRecovered,
    TransactionSignedNoHash,
};
use reth_rpc_types::CallRequest;
use revm::primitives::{
    AccountInfo as ReVmAccountInfo, BlockEnv as ReVmBlockEnv, CreateScheme, TransactTo, TxEnv, U256,
};

use super::primitive_types::{
    BlockEnv, RawEvmTxConversionError, RlpEvmTransaction, TransactionSignedAndRecovered,
};
use super::AccountInfo;

impl From<AccountInfo> for ReVmAccountInfo {
    fn from(info: AccountInfo) -> Self {
        Self {
            nonce: info.nonce,
            balance: info.balance,
            code: None,
            code_hash: info.code_hash,
        }
    }
}

impl From<ReVmAccountInfo> for AccountInfo {
    fn from(info: ReVmAccountInfo) -> Self {
        Self {
            balance: info.balance,
            code_hash: info.code_hash,
            nonce: info.nonce,
        }
    }
}

impl From<&BlockEnv> for ReVmBlockEnv {
    fn from(block_env: &BlockEnv) -> Self {
        Self {
            number: U256::from(block_env.number),
            coinbase: block_env.coinbase,
            timestamp: U256::from(block_env.timestamp),
            difficulty: U256::ZERO,
            prevrandao: Some(block_env.prevrandao),
            basefee: U256::from(block_env.basefee),
            gas_limit: U256::from(block_env.gas_limit),
            // EIP-4844 related field
            // https://github.com/Sovereign-Labs/sovereign-sdk/issues/912
            blob_excess_gas_and_price: None,
        }
    }
}

pub(crate) fn create_tx_env(tx: &TransactionSignedEcRecovered) -> TxEnv {
    let to = match tx.to() {
        Some(addr) => TransactTo::Call(addr),
        None => TransactTo::Create(CreateScheme::Create),
    };

    TxEnv {
        caller: tx.signer(),
        gas_limit: tx.gas_limit(),
        gas_price: U256::from(tx.effective_gas_price(None)),
        gas_priority_fee: tx.max_priority_fee_per_gas().map(U256::from),
        transact_to: to,
        value: U256::from(tx.value()),
        data: Bytes::from(tx.input().to_vec()),
        chain_id: tx.chain_id(),
        nonce: Some(tx.nonce()),
        // TODO handle access list
        access_list: vec![],
        // EIP-4844 related fields
        // https://github.com/Sovereign-Labs/sovereign-sdk/issues/912
        blob_hashes: vec![],
        max_fee_per_blob_gas: None,
    }
}

impl TryFrom<RlpEvmTransaction> for TransactionSignedNoHash {
    type Error = RawEvmTxConversionError;

    fn try_from(data: RlpEvmTransaction) -> Result<Self, Self::Error> {
        let data = RethBytes::from(data.rlp);
        if data.is_empty() {
            return Err(RawEvmTxConversionError::EmptyRawTransactionData);
        }

        let transaction = TransactionSigned::decode_enveloped(data)
            .map_err(|_| RawEvmTxConversionError::FailedToDecodeSignedTransaction)?;

        Ok(transaction.into())
    }
}

impl TryFrom<RlpEvmTransaction> for TransactionSignedEcRecovered {
    type Error = RawEvmTxConversionError;

    fn try_from(evm_tx: RlpEvmTransaction) -> Result<Self, Self::Error> {
        let tx = TransactionSignedNoHash::try_from(evm_tx)?;
        let tx: TransactionSigned = tx.into();
        let tx = tx
            .into_ecrecovered()
            .ok_or(RawEvmTxConversionError::FailedToDecodeSignedTransaction)?;

        Ok(tx)
    }
}

impl From<TransactionSignedAndRecovered> for TransactionSignedEcRecovered {
    fn from(value: TransactionSignedAndRecovered) -> Self {
        TransactionSignedEcRecovered::from_signed_transaction(
            value.signed_transaction,
            value.signer,
        )
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
        access_list: request
            .access_list
            .map(AccessList::flattened)
            .unwrap_or_default(),
        // EIP-4844 related fields
        // https://github.com/Sovereign-Labs/sovereign-sdk/issues/912
        blob_hashes: request.blob_versioned_hashes,
        max_fee_per_blob_gas: request.max_fee_per_blob_gas,
    }
}
