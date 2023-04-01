use sov_app_template::{RawTx, TxVerifier};
use sov_modules_api::{Context, Signature};
use sovereign_sdk::jmt::SimpleHasher;
use sovereign_sdk::serial::Decode;
use std::{io::Cursor, marker::PhantomData};

/// Transaction represents a deserialized RawTx.
#[derive(Debug, PartialEq, Eq, Clone, borsh::BorshDeserialize, borsh::BorshSerialize)]
pub(crate) struct Transaction<C: sov_modules_api::Context> {
    pub(crate) signature: C::Signature,
    pub(crate) pub_key: C::PublicKey,
    pub(crate) runtime_msg: Vec<u8>,
    pub(crate) nonce: u64,
}

impl<C: sov_modules_api::Context> Transaction<C> {
    pub fn new(msg: Vec<u8>, pub_key: C::PublicKey, signature: C::Signature, nonce: u64) -> Self {
        Self {
            signature,
            runtime_msg: msg,
            pub_key,
            nonce,
        }
    }
}

pub(crate) struct DemoAppTxVerifier<C: Context> {
    _phantom: PhantomData<C>,
}

impl<C: Context> DemoAppTxVerifier<C> {
    pub fn new() -> Self {
        Self {
            _phantom: PhantomData,
        }
    }
}

impl<C: Context> TxVerifier for DemoAppTxVerifier<C> {
    type Transaction = Transaction<C>;

    fn verify_tx_stateless(&self, raw_tx: RawTx) -> anyhow::Result<Self::Transaction> {
        let mut data = Cursor::new(&raw_tx.data);
        let tx = Transaction::<C>::decode(&mut data)?;

        // We check signature against runtime_msg and nonce.
        let mut hasher = C::Hasher::new();
        hasher.update(&tx.runtime_msg);
        hasher.update(&tx.nonce.to_le_bytes());
        let msg_hash = hasher.finalize();

        tx.signature.verify(&tx.pub_key, msg_hash)?;

        Ok(tx)
    }
}
