#![deny(missing_docs)]
#![doc = include_str!("../README.md")]

use const_rollup_config::{ROLLUP_BATCH_NAMESPACE_RAW, ROLLUP_PROOF_NAMESPACE_RAW};
use sov_celestia_adapter::types::Namespace;
mod mock_rollup;
pub use mock_rollup::*;
mod celestia_rollup;
pub use celestia_rollup::*;
#[cfg(feature = "experimental")]
mod eth;

/// The rollup stores its data in the namespace b"sov-test" on Celestia
/// You can change this constant to point your rollup at a different namespace
pub const ROLLUP_BATCH_NAMESPACE: Namespace = Namespace::const_v0(ROLLUP_BATCH_NAMESPACE_RAW);

/// The rollup stores the zk proofs in the namespace b"sov-test-p" on Celestia.
pub const ROLLUP_PROOF_NAMESPACE: Namespace = Namespace::const_v0(ROLLUP_PROOF_NAMESPACE_RAW);
