use bytes::Bytes;
use ethers_core::types::{Bytes as EthBytes, OtherFields, Transaction};
use reth_primitives::{
    Bytes as RethBytes, TransactionSigned as RethTransactionSigned,
    TransactionSignedEcRecovered as RethTransactionSignedEcRecovered,
    TransactionSignedNoHash as RethTransactionSignedNoHash,
};
use reth_rpc::eth::error::EthApiError;
use reth_rpc_types::CallRequest;
use revm::primitives::{
    AccountInfo as ReVmAccountInfo, BlockEnv as ReVmBlockEnv, Bytecode, CreateScheme, TransactTo,
    TxEnv, B160, B256, U256,
};
use thiserror::Error;

use super::transaction::{BlockEnv, EvmTransactionSignedEcRecovered, RawEvmTransaction};
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
            number: U256::from(block_env.number),
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

impl From<&EvmTransactionSignedEcRecovered> for TxEnv {
    fn from(tx: &EvmTransactionSignedEcRecovered) -> Self {
        let tx: &RethTransactionSignedEcRecovered = tx.as_ref();

        let to = match tx.to() {
            Some(addr) => TransactTo::Call(addr),
            None => TransactTo::Create(CreateScheme::Create),
        };

        Self {
            caller: tx.signer(),
            gas_limit: tx.gas_limit(),
            gas_price: U256::from(tx.effective_gas_price(None)),
            gas_priority_fee: tx.max_priority_fee_per_gas().map(U256::from),
            transact_to: to,
            value: U256::from(tx.value()),
            data: Bytes::from(tx.input().to_vec()),
            chain_id: tx.chain_id(),
            nonce: Some(tx.nonce()),
            //TODO
            access_list: vec![],
        }
    }
}

impl TryFrom<RawEvmTransaction> for Transaction {
    type Error = RawEvmTxConversionError;
    fn try_from(evm_tx: RawEvmTransaction) -> Result<Self, Self::Error> {
        let tx: EvmTransactionSignedEcRecovered = evm_tx.try_into()?;
        let tx: &RethTransactionSignedEcRecovered = tx.as_ref();

        Ok(Self {
            hash: tx.hash().into(),
            nonce: tx.nonce().into(),

            from: tx.signer().into(),
            to: tx.to().map(|addr| addr.into()),
            value: tx.value().into(),
            gas_price: Some(tx.effective_gas_price(None).into()),

            input: EthBytes::from(tx.input().to_vec()),
            // TODO
            v: tx.signature().v(None).into(),
            r: tx.signature().r.into(),
            s: tx.signature().s.into(),
            transaction_type: Some(1u64.into()),
            // TODO
            access_list: None,
            max_priority_fee_per_gas: tx.max_priority_fee_per_gas().map(From::from),
            max_fee_per_gas: Some(tx.max_fee_per_gas().into()),
            chain_id: tx.chain_id().map(|id| id.into()),
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
        })
    }
}

#[derive(Error, Debug)]
pub enum RawEvmTxConversionError {
    #[error("Empty raw transaction data")]
    EmptyRawTransactionData,
    #[error("Failed to decode signed transaction")]
    FailedToDecodeSignedTransaction,
}

impl From<RawEvmTxConversionError> for EthApiError {
    fn from(e: RawEvmTxConversionError) -> Self {
        match e {
            RawEvmTxConversionError::EmptyRawTransactionData => {
                EthApiError::EmptyRawTransactionData
            }
            RawEvmTxConversionError::FailedToDecodeSignedTransaction => {
                EthApiError::FailedToDecodeSignedTransaction
            }
        }
    }
}

impl TryFrom<RawEvmTransaction> for RethTransactionSignedNoHash {
    type Error = RawEvmTxConversionError;

    fn try_from(data: RawEvmTransaction) -> Result<Self, Self::Error> {
        let data = RethBytes::from(data.tx);
        if data.is_empty() {
            return Err(RawEvmTxConversionError::EmptyRawTransactionData);
        }

        let transaction = RethTransactionSigned::decode_enveloped(data)
            .map_err(|_| RawEvmTxConversionError::FailedToDecodeSignedTransaction)?;

        Ok(transaction.into())
    }
}

impl TryFrom<RawEvmTransaction> for EvmTransactionSignedEcRecovered {
    type Error = RawEvmTxConversionError;

    fn try_from(evm_tx: RawEvmTransaction) -> Result<Self, Self::Error> {
        let tx = RethTransactionSignedNoHash::try_from(evm_tx)?;
        let tx: RethTransactionSigned = tx.into();
        let tx = tx
            .into_ecrecovered()
            .ok_or(RawEvmTxConversionError::FailedToDecodeSignedTransaction)?;

        Ok(EvmTransactionSignedEcRecovered::new(tx))
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
