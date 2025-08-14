#![deny(missing_docs)]
#![doc = include_str!("../README.md")]

use std::collections::HashMap;

use reth_primitives::{sign_message, Address, TransactionSigned};
use reth_primitives::{Transaction, TxEip1559, TxEip2930, TxEip4844, TxLegacy};
use reth_rpc_types::TypedTransactionRequest;
use revm::primitives::B256;
use secp256k1::PublicKey;
pub use secp256k1::SecretKey;

/// Ethereum transaction signer.
#[derive(Clone)]
pub struct Signer(SecretKey);

/// Signature error.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Error occurred while trying to sign data.
    #[error("Could not sign")]
    CouldNotSign,
    /// Signer for a requested account is not found.
    #[error("Unknown account")]
    NoAccount,
    /// Invalid transaction request in `sign_transaction`.
    #[error("invalid transaction request")]
    InvalidTransactionRequest,
}

impl Signer {
    /// Creates a new Signer.
    pub fn new(key: SecretKey) -> Self {
        Self(key)
    }

    /// Public key
    pub fn public_key(&self) -> PublicKey {
        PublicKey::from_secret_key(secp256k1::SECP256K1, &self.0)
    }

    /// Address
    pub fn address(&self) -> Address {
        reth_primitives::public_key_to_address(self.public_key())
    }

    /// Signs an ethereum transaction.
    pub fn sign_transaction(
        &self,
        request: TypedTransactionRequest,
    ) -> Result<TransactionSigned, Error> {
        let transaction =
            to_primitive_transaction(request).ok_or(Error::InvalidTransactionRequest)?;
        let tx_signature_hash = transaction.signature_hash();
        let sk = B256::from_slice(self.0.as_ref());
        let signature = sign_message(sk, tx_signature_hash).map_err(|_| Error::CouldNotSign)?;

        Ok(TransactionSigned::from_transaction_and_signature(
            transaction,
            signature,
        ))
    }
}

/// Ethereum transaction signer supporting multiple accounts.
#[derive(Clone)]
pub struct Signers(HashMap<Address, Signer>);

impl Signers {
    /// Creates a new Signer.
    pub fn new(keys: impl IntoIterator<Item = SecretKey>) -> Self {
        let signers = keys
            .into_iter()
            .map(|sk| {
                let signer = Signer::new(sk);
                (signer.address(), signer)
            })
            .collect();
        Self(signers)
    }

    /// Signs an ethereum transaction with a provided account.
    pub fn sign_transaction(
        &self,
        request: TypedTransactionRequest,
        address: &Address,
    ) -> Result<TransactionSigned, Error> {
        let signer = self.0.get(address).ok_or(Error::NoAccount)?;
        signer.sign_transaction(request)
    }

    /// List of signers.
    pub fn addresses(&self) -> Vec<Address> {
        self.0.keys().cloned().collect()
    }
}

/// Converts a typed transaction request into a primitive transaction.
///
/// Returns `None` if any of the following are true:
/// - `nonce` is greater than [`u64::MAX`]
/// - `gas_limit` is greater than [`u64::MAX`]
/// - `value` is greater than [`u128::MAX`]
///   Copy from [`reth_rpc_types_compat::transaction::to_primitive_transaction`]
pub fn to_primitive_transaction(tx_request: TypedTransactionRequest) -> Option<Transaction> {
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
