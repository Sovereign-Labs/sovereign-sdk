use std::collections::HashMap;

use reth_primitives::{sign_message, Address, Transaction, TransactionSigned, H256};
use secp256k1::{PublicKey, SecretKey};

/// Ethereum transaction signer.
pub struct DevSigner {
    signers: HashMap<Address, SecretKey>,
}

#[derive(Debug, thiserror::Error)]
pub enum SignError {
    /// Error occured while trying to sign data.
    #[error("Could not sign")]
    CouldNotSign,
    /// Signer for requested account not found.
    #[error("Unknown account")]
    NoAccount,
    /// TypedData has invalid format.
    #[error("Given typed data is not valid")]
    TypedData,
    /// No chainid
    #[error("No chainid")]
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
        transaction: Transaction,
        address: Address,
    ) -> Result<TransactionSigned, SignError> {
        let tx_signature_hash = transaction.signature_hash();
        let signer = self.signers.get(&address).ok_or(SignError::NoAccount)?;

        let signature = sign_message(H256::from_slice(signer.as_ref()), tx_signature_hash)
            .map_err(|_| SignError::CouldNotSign)?;

        Ok(TransactionSigned::from_transaction_and_signature(
            transaction,
            signature,
        ))
    }

    pub fn signers(&self) -> Vec<Address> {
        self.signers.keys().copied().collect()
    }
}
