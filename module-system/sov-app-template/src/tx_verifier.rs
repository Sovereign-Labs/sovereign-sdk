use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use sov_modules_api::{Context, Hasher, Spec};

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

/// TxVerifier encapsulates Transaction verification.
pub trait TxVerifier {
    type Transaction;

    /// Runs stateless checks against a single RawTx.
    fn verify_tx_stateless(&self, raw_tx: RawTx) -> anyhow::Result<Self::Transaction>;

    /// Runs stateless checks against RawTxs.
    /// Returns verified transaction and hash of the RawTx.
    fn verify_txs_stateless<C: Context>(
        &self,
        raw_txs: Vec<RawTx>,
    ) -> anyhow::Result<Vec<(Self::Transaction, RawTxHash)>> {
        let mut txs = Vec::with_capacity(raw_txs.len());
        for raw_tx in raw_txs {
            let raw_tx_hash = raw_tx.hash::<C>();
            let tx = self.verify_tx_stateless(raw_tx)?;

            txs.push((tx, raw_tx_hash));
        }

        Ok(txs)
    }
}
