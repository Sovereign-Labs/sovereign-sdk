#![deny(missing_docs)]
#![doc = include_str!("../README.md")]

use risc0_zkvm::sha::Digest;
use serde::{Deserialize, Serialize};
use sov_rollup_interface::zk::Matches;

/// Guest or code that runs inside ZkVM
pub mod guest;

#[cfg(feature = "native")]
/// Host or code that runs outside ZkVM and interacts with the guest
pub mod host;

#[cfg(feature = "bench")]
pub mod metrics;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// Risc0 implementation of a commitment to the zkVM program which is being proven
pub struct Risc0MethodId([u32; 8]);

impl Matches<Self> for Risc0MethodId {
    fn matches(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Matches<Digest> for Risc0MethodId {
    fn matches(&self, other: &Digest) -> bool {
        self.0 == other.as_words()
    }
}

impl Matches<[u32; 8]> for Risc0MethodId {
    fn matches(&self, other: &[u32; 8]) -> bool {
        &self.0 == other
    }
}
