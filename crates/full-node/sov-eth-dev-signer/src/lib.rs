#![deny(missing_docs)]
#![doc = include_str!("../README.md")]

use std::collections::HashMap;

use alloy_consensus::SignableTransaction;
use alloy_consensus::{TxEip4844Variant, TypedTransaction};
use reth_primitives::{sign_message, Transaction, TransactionSigned};
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
        request: TypedTransaction,
        address: Address,
    ) -> Result<TransactionSigned, SignError> {
        let transaction =
            to_primitive_transaction(request).ok_or(SignError::InvalidTransactionRequest)?;
        let tx_signature_hash = transaction.signature_hash();
        let signer = self.signers.get(&address).ok_or(SignError::NoAccount)?;

        let signature = sign_message(B256::from_slice(signer.as_ref()), tx_signature_hash)
            .map_err(|_| SignError::CouldNotSign)?;

        Ok(TransactionSigned::new_unhashed(transaction, signature))
    }

    /// List of signers.
    pub fn signers(&self) -> Vec<Address> {
        self.signers.keys().copied().collect()
    }
}

/// Converts a typed transaction request into a primitive transaction.
fn to_primitive_transaction(tx_request: TypedTransaction) -> Option<Transaction> {
    Some(match tx_request {
        TypedTransaction::Legacy(tx) => Transaction::Legacy(tx),
        TypedTransaction::Eip2930(tx) => Transaction::Eip2930(tx),
        TypedTransaction::Eip1559(tx) => Transaction::Eip1559(tx),
        TypedTransaction::Eip4844(TxEip4844Variant::TxEip4844(tx)) => Transaction::Eip4844(tx),
        TypedTransaction::Eip4844(TxEip4844Variant::TxEip4844WithSidecar(tx)) => {
            Transaction::Eip4844(tx.into())
        }
        TypedTransaction::Eip7702(tx) => Transaction::Eip7702(tx),
    })
}
