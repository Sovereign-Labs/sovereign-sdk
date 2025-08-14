//! Ethereum transaction signer.
use std::collections::HashMap;

use reth_primitives::{sign_message, Address, TransactionSigned};
use reth_rpc_types::TypedTransactionRequest;
use revm::primitives::B256;
use secp256k1::PublicKey;
pub use secp256k1::SecretKey;

use crate::transaction::to_primitive_transaction;

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
