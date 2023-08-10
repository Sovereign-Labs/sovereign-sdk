use ethers_core::rand::rngs::StdRng;
use ethers_core::rand::SeedableRng;
use reth_primitives::{
    public_key_to_address, sign_message, Bytes as RethBytes, Transaction as RethTransaction,
    TransactionKind, TransactionSigned, TxEip1559 as RethTxEip1559, H256,
};
use reth_rpc::eth::error::SignError;
use secp256k1::{PublicKey, SecretKey};

use crate::evm::{EthAddress, RawEvmTransaction};

/// ETH transactions signer used in tests.
pub(crate) struct DevSigner {
    secret_key: SecretKey,
    pub(crate) address: EthAddress,
}

impl DevSigner {
    /// Creates a new signer.
    pub(crate) fn new(secret_key: SecretKey) -> Self {
        let public_key = PublicKey::from_secret_key(secp256k1::SECP256K1, &secret_key);
        let addr = public_key_to_address(public_key);
        Self {
            secret_key,
            address: addr.into(),
        }
    }

    /// Creates a new signer with random private key.
    pub(crate) fn new_random() -> Self {
        let mut rng = StdRng::seed_from_u64(22);
        let secret_key = SecretKey::new(&mut rng);
        Self::new(secret_key)
    }

    /// Signs Eip1559 transaction.
    pub(crate) fn sign_transaction(
        &self,
        transaction: RethTxEip1559,
    ) -> Result<TransactionSigned, SignError> {
        let transaction = RethTransaction::Eip1559(transaction);

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

    /// Signs default Eip1559 transaction with to, data and nonce overridden.
    pub(crate) fn sign_default_transaction(
        &self,
        to: TransactionKind,
        data: Vec<u8>,
        nonce: u64,
    ) -> Result<RawEvmTransaction, SignError> {
        let reth_tx = RethTxEip1559 {
            to,
            input: RethBytes::from(data),
            nonce,
            chain_id: 1,
            gas_limit: u64::MAX,
            ..Default::default()
        };

        let signed = self.sign_transaction(reth_tx)?;

        Ok(RawEvmTransaction {
            tx: signed.envelope_encoded().to_vec(),
        })
    }
}
