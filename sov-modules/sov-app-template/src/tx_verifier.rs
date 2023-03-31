use borsh::{BorshDeserialize, BorshSerialize};

use sov_modules_api::Context;

/// RawTx represents a serialized rollup transaction received from the DA.
#[derive(Debug, PartialEq, BorshDeserialize, BorshSerialize, Clone)]
pub struct RawTx {
    pub data: Vec<u8>,
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
pub struct VerifiedTx<C: Context> {
    pub pub_key: C::PublicKey,
    pub sender: C::Address,
    pub runtime_msg: Vec<u8>,
}

/// TxVerifier encapsulates Transaction verification.
pub trait TxVerifier {
    type Context: Context;

    /// Runs stateless checks against a single RawTx.
    fn verify_tx_stateless(&self, raw_tx: RawTx) -> anyhow::Result<Transaction<Self::Context>>;

    /// Runs stateless checks against RawTxs.
    fn verify_txs_stateless(
        &self,
        raw_txs: Vec<RawTx>,
    ) -> anyhow::Result<Vec<Transaction<Self::Context>>> {
        let mut txs = Vec::with_capacity(raw_txs.len());
        for raw_tx in raw_txs {
            let tx = self.verify_tx_stateless(raw_tx)?;
            txs.push(tx);
        }

        Ok(txs)
    }
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
