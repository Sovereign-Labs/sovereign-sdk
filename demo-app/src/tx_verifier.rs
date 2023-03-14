use borsh::{BorshDeserialize, BorshSerialize};
use sov_modules_api::{Context, Signature, Spec};
use sovereign_sdk::jmt::SimpleHasher;
use sovereign_sdk::serial::Decode;
use std::{io::Cursor, marker::PhantomData};

/// RawTx represents a serialized rollup transaction received from the DA.
#[derive(Debug, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct RawTx {
    pub(crate) data: Vec<u8>,
}

/// Transaction represents a deserialized RawTx.
#[derive(Debug, PartialEq, Eq, Clone, BorshDeserialize, BorshSerialize)]
pub struct Transaction<C: sov_modules_api::Context> {
    pub signature: C::Signature,
    pub pub_key: C::PublicKey,
    pub runtime_msg: Vec<u8>,
    pub nonce: u64,
}

/// VerifiedTx is a Transaction after verification.
pub(crate) struct VerifiedTx<C: Context> {
    pub(crate) sender: C::PublicKey,
    pub(crate) runtime_msg: Vec<u8>,
    pub(crate) _nonce: u64,
}

/// TxVerifier encapsulates Transaction verification.
pub(crate) trait TxVerifier {
    type Context: Context;

    /// Runs stateless checks against RawTxs.
    fn verify_txs_stateless(
        &self,
        txs: Vec<RawTx>,
    ) -> anyhow::Result<Vec<Transaction<Self::Context>>>;

    /// Runs stateful checks against a Transaction. This method can modify the storage.
    fn verify_tx_stateful(
        &self,
        tx: Transaction<Self::Context>,
        storage: <Self::Context as Spec>::Storage,
    ) -> anyhow::Result<VerifiedTx<Self::Context>>;
}

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

    fn verify_txs_stateless(
        &self,
        raw_txs: Vec<RawTx>,
    ) -> anyhow::Result<Vec<Transaction<Self::Context>>> {
        let mut txs = Vec::with_capacity(raw_txs.len());

        for raw_tx in raw_txs {
            let mut data = Cursor::new(&raw_tx.data);
            let tx = Transaction::<C>::decode(&mut data)?;

            // We check signature against runtime_msg and nonce.
            let mut hasher = C::Hasher::new();
            hasher.update(&tx.runtime_msg);
            hasher.update(&tx.nonce.to_le_bytes());
            let msg_hash = hasher.finalize();

            tx.signature.verify(&tx.pub_key, msg_hash)?;
            txs.push(tx)
        }

        Ok(txs)
    }

    fn verify_tx_stateful(
        &self,
        tx: Transaction<Self::Context>,
        _storage: <Self::Context as Spec>::Storage,
    ) -> anyhow::Result<VerifiedTx<Self::Context>> {
        // TODO add stateful checks: account existence, nonce, etc..

        Ok(VerifiedTx {
            sender: tx.pub_key,
            runtime_msg: tx.runtime_msg,
            _nonce: tx.nonce,
        })
    }
}
