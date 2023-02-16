use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

// Represents a candidate.
#[derive(BorshDeserialize, BorshSerialize, Serialize, Deserialize, Debug, Eq, PartialEq, Clone)]
pub struct Candidate {
    pub name: String,
    pub count: u32,
}

impl Candidate {
    pub fn new(name: String) -> Self {
        Self { name, count: 0 }
    }
}

/// Represents a voter.
#[derive(BorshDeserialize, BorshSerialize, Clone)]
pub(crate) enum Voter {
    Fresh,
    Voted,
}

impl Voter {
    pub(crate) fn fresh() -> Self {
        Self::Fresh
    }

    pub(crate) fn voted() -> Self {
        Self::Voted
    }
}
