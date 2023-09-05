//! Defines the database used by the Sovereign SDK.
//!
//! - Types and traits for storing and retrieving ledger data can be found in the [`ledger_db`] module
//! - DB "Table" definitions can be found in the [`schema`] module
//! - Types and traits for storing state data can be found in the [`state_db`] module
//! - The default db configuration is generated in the [`rocks_db_config`] module
#![forbid(unsafe_code)]
#![deny(missing_docs)]

/// Implements a wrapper around RocksDB meant for storing rollup history ("the ledger").
/// This wrapper implements helper traits for writing blocks to the ledger, and for
/// serving historical data via RPC
pub mod ledger_db;
/// Implements helpers for configuring RocksDB.
pub mod rocks_db_config;
/// Defines the tables used by the Sovereign SDK.
pub mod schema;
/// Implements a wrapper around RocksDB meant for storing rollup state. This is primarily used
/// as the backing store for the JMT.
pub mod state_db;

/// Implements a wrapper around RocksDB meant for storing state only accessible
/// outside of the zkVM execution environment, as this data is not included in
/// the JMT and does not contribute to proofs of execution.
pub mod native_db;
