#![deny(missing_docs)]
#![doc = include_str!("../README.md")]

use std::collections::HashMap;

use reth_primitives::{
    sign_message, Transaction, TransactionSigned, TxEip1559, TxEip2930, TxEip4844, TxLegacy,
};
use reth_rpc_types::TypedTransactionRequest;
use revm::primitives::{Address, B256};
use secp256k1::{PublicKey, SecretKey};

/// Ethereum transaction signer.
#[derive(Clone)]
pub struct DevSigner {
    signers: HashMap<Address, SecretKey>,
}

/// Signature error.
#[derive(Debug, thiserror::Error)]
pub enum SignError {
    /// Error occurred while trying to sign data.
    #[error("Could not sign")]
    CouldNotSign,
    /// Signer for a requested account is not found.
    #[error("Unknown account")]
    NoAccount,
    /// TypedData has an invalid format.
    #[error("Given typed data is not valid")]
    TypedData,
    /// Invalid transaction request in `sign_transaction`.
    #[error("invalid transaction request")]
    InvalidTransactionRequest,
    /// No chain id
    #[error("No chain id")]
    NoChainId,
}

impl DevSigner {
    /// Creates a new DevSigner.
    pub fn new(secret_keys: Vec<SecretKey>) -> Self {
        let mut signers = HashMap::with_capacity(secret_keys.len());

        for sk in secret_keys {
            let public_key = PublicKey::from_secret_key(secp256k1::SECP256K1, &sk);
            let address = reth_primitives::public_key_to_address(public_key);

            signers.insert(address, sk);
        }

        Self { signers }
    }

    /// Signs an ethereum transaction.
    pub fn sign_transaction(
        &self,
        request: TypedTransactionRequest,
        address: Address,
    ) -> Result<TransactionSigned, SignError> {
        let transaction =
            to_primitive_transaction(request).ok_or(SignError::InvalidTransactionRequest)?;
        let tx_signature_hash = transaction.signature_hash();
        let signer = self.signers.get(&address).ok_or(SignError::NoAccount)?;

        let signature = sign_message(B256::from_slice(signer.as_ref()), tx_signature_hash)
            .map_err(|_| SignError::CouldNotSign)?;

        Ok(TransactionSigned::from_transaction_and_signature(
            transaction,
            signature,
        ))
    }

    /// List of signers.
    pub fn signers(&self) -> Vec<Address> {
        self.signers.keys().copied().collect()
    }
}

/// Converts a typed transaction request into a primitive transaction.
///
/// Returns `None` if any of the following are true:
/// - `nonce` is greater than [`u64::MAX`]
/// - `gas_limit` is greater than [`u64::MAX`]
/// - `value` is greater than [`u128::MAX`]
///   Copy from [`reth_rpc_types_compat::transaction::to_primitive_transaction`]
fn to_primitive_transaction(tx_request: TypedTransactionRequest) -> Option<Transaction> {
    Some(match tx_request {
        TypedTransactionRequest::Legacy(tx) => Transaction::Legacy(TxLegacy {
            chain_id: tx.chain_id,
            nonce: tx.nonce,
            gas_price: tx.gas_price.to(),
            gas_limit: tx.gas_limit.try_into().ok()?,
            to: tx.kind,
            value: tx.value,
            input: tx.input,
        }),
        TypedTransactionRequest::EIP2930(tx) => Transaction::Eip2930(TxEip2930 {
            chain_id: tx.chain_id,
            nonce: tx.nonce,
            gas_price: tx.gas_price.to(),
            gas_limit: tx.gas_limit.try_into().ok()?,
            to: tx.kind,
            value: tx.value,
            input: tx.input,
            access_list: tx.access_list,
        }),
        TypedTransactionRequest::EIP1559(tx) => Transaction::Eip1559(TxEip1559 {
            chain_id: tx.chain_id,
            nonce: tx.nonce,
            max_fee_per_gas: tx.max_fee_per_gas.to(),
            gas_limit: tx.gas_limit.try_into().ok()?,
            to: tx.kind,
            value: tx.value,
            input: tx.input,
            access_list: tx.access_list,
            max_priority_fee_per_gas: tx.max_priority_fee_per_gas.to(),
        }),
        TypedTransactionRequest::EIP4844(tx) => Transaction::Eip4844(TxEip4844 {
            chain_id: tx.chain_id,
            nonce: tx.nonce,
            gas_limit: tx.gas_limit.to(),
            max_fee_per_gas: tx.max_fee_per_gas.to(),
            max_priority_fee_per_gas: tx.max_priority_fee_per_gas.to(),
            placeholder: None,
            to: tx.to,
            value: tx.value,
            access_list: tx.access_list,
            blob_versioned_hashes: tx.blob_versioned_hashes,
            max_fee_per_blob_gas: tx.max_fee_per_blob_gas.to(),
            input: tx.input,
        }),
    })
}
