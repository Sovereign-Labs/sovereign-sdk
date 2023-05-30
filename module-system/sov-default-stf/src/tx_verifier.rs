use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use sov_modules_api::Signature;
use sov_modules_api::{transaction::Transaction, Context, Hasher, Spec};
use std::io::Cursor;
use tracing::debug;
/// RawTx represents a serialized rollup transaction received from the DA.
#[derive(Debug, PartialEq, Clone, BorshDeserialize, BorshSerialize, Serialize, Deserialize)]
pub struct RawTx {
    pub data: Vec<u8>,
}

impl RawTx {
    fn hash<C: Context>(&self) -> [u8; 32] {
        <C as Spec>::Hasher::hash(&self.data)
    }
}

type RawTxHash = [u8; 32];

pub fn verify_txs_stateless<C: Context>(
    raw_txs: Vec<RawTx>,
) -> anyhow::Result<Vec<(Transaction<C>, RawTxHash)>> {
    let mut txs = Vec::with_capacity(raw_txs.len());
    debug!("Verifying {} transactions", raw_txs.len());
    for raw_tx in raw_txs {
        let raw_tx_hash = raw_tx.hash::<C>();
        let tx = verify_tx_stateless(raw_tx)?;

        txs.push((tx, raw_tx_hash));
    }

    Ok(txs)
}

fn verify_tx_stateless<C: Context>(raw_tx: RawTx) -> anyhow::Result<Transaction<C>> {
    let mut data = Cursor::new(&raw_tx.data);
    let tx = Transaction::<C>::deserialize_reader(&mut data)?;

    // We check signature against runtime_msg and nonce.
    let mut hasher = C::Hasher::new();
    hasher.update(tx.runtime_msg());
    hasher.update(&tx.nonce().to_le_bytes());

    let msg_hash = hasher.finalize();
    tx.signature().verify(tx.pub_key(), msg_hash)?;

    Ok(tx)
}
