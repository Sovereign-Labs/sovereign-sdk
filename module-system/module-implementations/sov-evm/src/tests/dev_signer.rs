use ethers_core::rand::rngs::StdRng;
use ethers_core::rand::SeedableRng;
use reth_primitives::{
    Address, Bytes as RethBytes, Transaction as RethTransaction, TransactionKind,
    TxEip1559 as RethTxEip1559,
};
use secp256k1::{PublicKey, SecretKey};

use crate::evm::RlpEvmTransaction;
use crate::signer::{DevSigner, SignError};

/// ETH transactions signer used in tests.
pub(crate) struct TestSigner {
    signer: DevSigner,
    address: Address,
}

impl TestSigner {
    /// Creates a new signer.
    pub(crate) fn new(secret_key: SecretKey) -> Self {
        let public_key = PublicKey::from_secret_key(secp256k1::SECP256K1, &secret_key);
        let address = reth_primitives::public_key_to_address(public_key);
        Self {
            signer: DevSigner::new(vec![secret_key]),
            address,
        }
    }

    /// Creates a new signer with random private key.
    pub(crate) fn new_random() -> Self {
        let mut rng = StdRng::seed_from_u64(22);
        let secret_key = SecretKey::new(&mut rng);
        Self::new(secret_key)
    }

    /// Address of the transaction signer.
    pub(crate) fn address(&self) -> Address {
        self.address
    }

    /// Signs default Eip1559 transaction with to, data and nonce overridden.
    pub(crate) fn sign_default_transaction(
        &self,
        to: TransactionKind,
        data: Vec<u8>,
        nonce: u64,
    ) -> Result<RlpEvmTransaction, SignError> {
        let reth_tx = RethTxEip1559 {
            to,
            input: RethBytes::from(data),
            nonce,
            chain_id: 1,
            gas_limit: 1_000_000u64,
            max_fee_per_gas: u128::from(reth_primitives::constants::MIN_PROTOCOL_BASE_FEE * 2),
            ..Default::default()
        };

        let reth_tx = RethTransaction::Eip1559(reth_tx);
        let signed = self.signer.sign_transaction(reth_tx, self.address)?;

        Ok(RlpEvmTransaction {
            rlp: signed.envelope_encoded().to_vec(),
        })
    }
}
