use ethers_core::rand::rngs::StdRng;
use ethers_core::rand::SeedableRng;
use reth_primitives::{
    Address, Bytes as RethBytes, Transaction as RethTransaction, TransactionKind,
    TxEip1559 as RethTxEip1559,
};
use reth_rpc::eth::error::SignError;
use secp256k1::SecretKey;

use crate::evm::RlpEvmTransaction;
use crate::sequencer::Signer;

/// ETH transactions signer used in tests.
pub(crate) struct DevSigner {
    signer: Signer,
}

impl DevSigner {
    /// Creates a new signer.
    pub(crate) fn new(secret_key: SecretKey) -> Self {
        Self {
            signer: Signer::new(secret_key),
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
        self.signer.address
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
            gas_limit: reth_primitives::constants::ETHEREUM_BLOCK_GAS_LIMIT / 2,
            ..Default::default()
        };

        let reth_tx = RethTransaction::Eip1559(reth_tx);
        let signed = self.signer.sign_transaction(reth_tx)?;

        Ok(RlpEvmTransaction {
            rlp: signed.envelope_encoded().to_vec(),
        })
    }
}
