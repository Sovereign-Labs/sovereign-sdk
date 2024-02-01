use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

use crate::tx_verifier::RawTx;

/// Contains raw transactions obtained from the DA blob.
#[derive(Debug, PartialEq, Clone, BorshDeserialize, BorshSerialize, Serialize, Deserialize)]
pub struct Batch {
    /// Raw transactions.
    pub txs: Vec<RawTx>,
}
