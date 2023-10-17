#![deny(missing_docs)]
//! # RISC0 Adapter
//!
//! This crate contains an adapter allowing the Risc0 to be used as a proof system for
//! Sovereign SDK rollups.
use risc0_zkvm::sha::Digest;
use serde::{Deserialize, Serialize};
use sov_rollup_interface::zk::Matches;

pub mod guest;
#[cfg(feature = "native")]
pub mod host;

#[cfg(feature = "bench")]
pub mod metrics;

/// Uniquely identifies a Risc0 binary. Roughly equivalent to
/// the hash of the ELF file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
