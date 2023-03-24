use borsh::{BorshDeserialize, BorshSerialize};

// Items that should go in prelude
use jmt::SimpleHasher;
use sov_modules_api::{mocks::MockContext, Spec};

use sovereign_sdk::{
    core::traits::{BatchTrait, CanonicalHash, TransactionTrait},
    jmt,
};

use crate::tx_verifier::RawTx;

#[derive(Debug, PartialEq, BorshDeserialize, BorshSerialize, Clone)]
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

impl CanonicalHash for RawTx {
    type Output = [u8; 32];

    fn hash(&self) -> Self::Output {
        <MockContext as Spec>::Hasher::hash(&self.data)
    }
}
