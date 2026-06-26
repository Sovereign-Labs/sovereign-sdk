//! Types, traits, or utilities that are used by the full node but are not part
//! of the rollup's state machine.
//!
//! This code is **never** used inside of zkVMs, so it can be non-deterministic,
//! access system resources or networking, write data to disk, etc..

pub mod da;
mod da_sync_state;
pub mod ledger_api;

pub use da_sync_state::SyncStatus;
