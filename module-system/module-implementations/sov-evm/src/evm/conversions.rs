use bytes::Bytes;
use ethers_core::types::Transaction;
use ethers_core::utils::rlp::{self, DecoderError};
use reth_rpc::eth::error::EthApiError;
use reth_rpc_types::CallRequest;
use revm::primitives::{AccountInfo as ReVmAccountInfo, Bytecode, TransactTo, TxEnv, B256, U256};
use thiserror::Error;

use super::transaction::RawEvmTransaction;
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

impl TryFrom<RawEvmTransaction> for Transaction {
    type Error = DecoderError;
    fn try_from(evm_tx: RawEvmTransaction) -> Result<Self, Self::Error> {
        rlp::decode::<Transaction>(&evm_tx.rlp)

        // Ok(Self {
        //     hash: tx.hash().into(),
        //     nonce: tx.nonce().into(),

        //     from: tx.signer().into(),
        //     to: tx.to().map(|addr| addr.into()),
        //     value: tx.value().into(),
        //     gas_price: Some(tx.effective_gas_price(None).into()),

        //     input: EthBytes::from(tx.input().to_vec()),
        //     v: tx.signature().v(tx.chain_id()).into(),
        //     r: tx.signature().r.into(),
        //     s: tx.signature().s.into(),
        //     transaction_type: Some(U64::from(tx.tx_type() as u8)),
        //     // TODO handle access list
        //     access_list: None,
        //     max_priority_fee_per_gas: tx.max_priority_fee_per_gas().map(From::from),
        //     max_fee_per_gas: Some(tx.max_fee_per_gas().into()),
        //     chain_id: tx.chain_id().map(|id| id.into()),
        //     // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/503
        //     block_hash: Some([0; 32].into()),
        //     // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/503
        //     block_number: Some(1.into()),
        //     // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/503
        //     transaction_index: Some(1.into()),
        //     // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/503
        //     gas: Default::default(),
        //     // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/503
        //     other: OtherFields::default(),
        // })
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
        // TODO handle access list
        access_list: Default::default(),
    }
}
