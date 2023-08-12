use std::io::Cursor;

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use sov_modules_api::transaction::Transaction;
use sov_modules_api::{Context, Spec};
use sov_rollup_interface::digest::Digest;
use tracing::debug;

type RawTxHash = [u8; 32];

pub(crate) struct TransactionAndRawHash<C: Context> {
    pub(crate) tx: Transaction<C>,
    pub(crate) raw_tx_hash: RawTxHash,
}

/// RawTx represents a serialized rollup transaction received from the DA.
#[derive(Debug, PartialEq, Clone, BorshDeserialize, BorshSerialize, Serialize, Deserialize)]
pub struct RawTx {
    /// Serialized transaction.
    pub data: Vec<u8>,
}

impl RawTx {
    fn hash<C: Context>(&self) -> [u8; 32] {
        <C as Spec>::Hasher::digest(&self.data).into()
    }
}

pub(crate) fn verify_txs_stateless<C: Context>(
    raw_txs: Vec<RawTx>,
) -> anyhow::Result<Vec<TransactionAndRawHash<C>>> {
    let mut txs = Vec::with_capacity(raw_txs.len());
    debug!("Verifying {} transactions", raw_txs.len());
    for raw_tx in raw_txs {
        let raw_tx_hash = raw_tx.hash::<C>();
        let mut data = Cursor::new(&raw_tx.data);
        let tx = Transaction::<C>::deserialize_reader(&mut data)?;
        tx.verify()?;
        txs.push(TransactionAndRawHash { tx, raw_tx_hash });
    }
    Ok(txs)
}
