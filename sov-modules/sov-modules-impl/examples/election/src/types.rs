use borsh::{BorshDeserialize, BorshSerialize};
#[cfg(feature = "native")]
use serde::{Deserialize, Serialize};

// Represents a candidate.
#[cfg_attr(feature = "native", derive(serde::Deserialize, serde::Serialize))]
#[derive(BorshDeserialize, BorshSerialize, Debug, Eq, PartialEq, Clone)]
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
