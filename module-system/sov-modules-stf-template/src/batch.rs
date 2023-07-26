use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

// use sov_rollup_interface::traits::TransactionTrait;
use crate::tx_verifier::RawTx;

#[derive(Debug, PartialEq, Clone, BorshDeserialize, BorshSerialize, Serialize, Deserialize)]
pub struct Batch {
    pub txs: Vec<RawTx>,
}

impl Batch {
    pub fn transactions(&self) -> &[RawTx] {
        &self.txs
    }

    pub fn take_transactions(self) -> Vec<RawTx> {
        self.txs
    }
}
