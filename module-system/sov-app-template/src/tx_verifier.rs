use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

/// RawTx represents a serialized rollup transaction received from the DA.
#[derive(Debug, PartialEq, Clone, BorshDeserialize, BorshSerialize, Serialize, Deserialize)]
pub struct RawTx {
    pub data: Vec<u8>,
}

/// TxVerifier encapsulates Transaction verification.
pub trait TxVerifier {
    type Transaction;

    /// Runs stateless checks against a single RawTx.
    fn verify_tx_stateless(&self, raw_tx: RawTx) -> anyhow::Result<Self::Transaction>;

    /// Runs stateless checks against RawTxs.
    fn verify_txs_stateless(&self, raw_txs: Vec<RawTx>) -> anyhow::Result<Vec<Self::Transaction>> {
        let mut txs = Vec::with_capacity(raw_txs.len());
        for raw_tx in raw_txs {
            let tx = self.verify_tx_stateless(raw_tx)?;
            txs.push(tx);
        }

        Ok(txs)
    }
}
