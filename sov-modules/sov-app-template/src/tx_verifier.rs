use borsh::{BorshDeserialize, BorshSerialize};
use sov_modules_api::Context;

/// RawTx represents a serialized rollup transaction received from the DA.
#[derive(Debug, PartialEq, BorshDeserialize, BorshSerialize, Clone)]
pub struct RawTx {
    pub data: Vec<u8>,
}

/// TxVerifier encapsulates Transaction verification.
pub trait TxVerifier {
    type Context: Context;
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
