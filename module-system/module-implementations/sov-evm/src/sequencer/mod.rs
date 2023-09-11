use std::str::FromStr;

use reth_primitives::{sign_message, Address, Transaction, TransactionSigned, H256};
use reth_rpc::eth::error::SignError;
use secp256k1::{PublicKey, SecretKey};

/// Ethereum transaction signer.
pub struct Signer {
    secret_key: SecretKey,
    pub address: Address,
}

impl Signer {
    /// Creates a new signer.
    pub fn new(secret_key: SecretKey) -> Self {
        let public_key = PublicKey::from_secret_key(secp256k1::SECP256K1, &secret_key);
        let addr = reth_primitives::public_key_to_address(public_key);
        Self {
            secret_key,
            address: addr,
        }
    }

    /// Signs an ethereum transaction.
    pub fn sign_transaction(
        &self,
        transaction: Transaction,
    ) -> Result<TransactionSigned, SignError> {
        let tx_signature_hash = transaction.signature_hash();

        let signature = sign_message(
            H256::from_slice(self.secret_key.as_ref()),
            tx_signature_hash,
        )
        .map_err(|_| SignError::CouldNotSign)?;

        Ok(TransactionSigned::from_transaction_and_signature(
            transaction,
            signature,
        ))
    }
}

impl FromStr for Signer {
    type Err = secp256k1::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let sk = SecretKey::from_str(s)?;
        Ok(Signer::new(sk))
    }
}
