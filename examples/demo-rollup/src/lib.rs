#![deny(missing_docs)]
#![doc = include_str!("../README.md")]

pub mod register_rpc;
mod rollup;

use const_rollup_config::ROLLUP_NAMESPACE_RAW;
pub use rollup::{
    new_rollup_with_celestia_da, new_rollup_with_mock_da, new_rollup_with_mock_da_from_config,
    DemoProverConfig, Rollup,
};
use sov_celestia_adapter::types::Namespace;
use sov_db::ledger_db::LedgerDB;

/// The rollup stores its data in the namespace b"sov-test" on Celestia
/// You can change this constant to point your rollup at a different namespace
pub const ROLLUP_NAMESPACE: Namespace = Namespace::const_v0(ROLLUP_NAMESPACE_RAW);

/// Initializes a [`LedgerDB`] using the provided `path`.
pub fn initialize_ledger(path: impl AsRef<std::path::Path>) -> LedgerDB {
    LedgerDB::with_path(path).expect("Ledger DB failed to open")
}
