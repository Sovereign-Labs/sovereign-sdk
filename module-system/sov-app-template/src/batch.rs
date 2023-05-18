use crate::tx_verifier::RawTx;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use sovereign_core::traits::{BatchTrait, TransactionTrait};

#[derive(Debug, PartialEq, Clone, BorshDeserialize, BorshSerialize, Serialize, Deserialize)]
pub struct Batch {
    pub txs: Vec<RawTx>,
}

impl BatchTrait for Batch {
    type Transaction = RawTx;

    fn transactions(&self) -> &[Self::Transaction] {
        &self.txs
    }

    fn take_transactions(self) -> Vec<Self::Transaction> {
        self.txs
    }
}

impl TransactionTrait for RawTx {
    type Hash = [u8; 32];
}
