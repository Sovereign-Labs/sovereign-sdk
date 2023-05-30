use borsh::BorshDeserialize;
use sov_default_stf::{RawTx, TxVerifier};
use sov_modules_api::{hooks::Transaction, Context, Hasher, Signature};
use std::{io::Cursor, marker::PhantomData};

pub struct DemoAppTxVerifier<C: Context> {
    _phantom: PhantomData<C>,
}

impl<C: Context> DemoAppTxVerifier<C> {
    #[allow(dead_code)]
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
        let tx = Transaction::<Self::Context>::deserialize_reader(&mut data)?;

        // We check signature against runtime_msg and nonce.
        let mut hasher = C::Hasher::new();
        hasher.update(tx.runtime_msg());
        hasher.update(&tx.nonce().to_le_bytes());

        let msg_hash = hasher.finalize();

        tx.signature().verify(&tx.pub_key(), msg_hash)?;

        Ok(tx)
    }
}
