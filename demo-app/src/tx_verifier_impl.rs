use sov_app_template::{RawTx, Transaction, TxVerifier};
use sov_modules_api::{Context, Signature};
use sovereign_sdk::jmt::SimpleHasher;
use sovereign_sdk::serial::Decode;
use std::{io::Cursor, marker::PhantomData};

pub(crate) struct DemoAppTxVerifier<C: Context> {
    // TODO add Accounts module for stateful checks.
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
    type Context = C;
    fn verify_tx_stateless(&self, raw_tx: RawTx) -> anyhow::Result<Transaction<Self::Context>> {
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
