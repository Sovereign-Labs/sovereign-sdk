//! Defines the database used by the Sovereign SDK.
//!
//! - Types and traits for storing and retrieving ledger data can be found in the [`ledger_db`] module
//! - DB "Table" definitions can be found in the [`schema`] module
//! - Types and traits for storing state data can be found in the [`state_db`] module
//! - The default db configuration is generated in the [`rocks_db_config`] module
#![forbid(unsafe_code)]
#![deny(missing_docs)]

use rockbound::{SchemaKey, SchemaValue};

/// Simpler version of `StateDb`, that stores key-values with versions for historical queries.
pub mod historical_state;
/// Implements a wrapper around RocksDB meant for storing rollup history ("the ledger").
/// This wrapper implements helper traits for writing blocks to the ledger, and for
/// serving historical data via RPC
pub mod ledger_db;
/// Implements helpers for configuring RocksDB.
pub mod rocks_db_config;
/// Defines the tables used by the Sovereign SDK.
pub mod schema;
/// Implements a wrapper around [RocksDB](https://rocksdb.org/) meant for storing rollup state.
/// This is primarily used as the backing store for the [JMT(JellyfishMerkleTree)](https://docs.rs/jmt/latest/jmt/).
pub mod state_db;

/// Implements a wrapper around RocksDB meant for storing state only accessible
/// outside of the zkVM execution environment, as this data is not included in
/// the JMT and does not contribute to proofs of execution.
pub mod accessory_db;

/// Define namespaces at the database level
pub mod namespaces;

/// Implements commit flag logic for state_db_nomt.
pub(crate) mod commit_flag;
/// Configuration for `sov-db`
pub mod config;
pub(crate) mod metrics;
/// Implements state pruning functionality for removing old versions of keys.
pub mod pruner;
/// Implements a wrapper around [NOMT](https://github.com/thrumdev/nomt) meant for storing rollup state.
pub mod state_db_nomt;
pub mod storage_manager;
/// Utils that are helpful outside the crate or for benchmarks.
#[cfg(any(test, feature = "test-utils"))]
pub mod test_utils;

/// Options on how to setup [`rockbound::DB`] or any other persistence.
pub struct DbOptions {
    /// Name of the database.
    pub(crate) name: &'static str,
    /// Sub-directory name for the [`rockbound::DB`].
    pub(crate) path_suffix: &'static str,
    /// A set of [`rockbound::schema::ColumnFamilyName`] that this db is going to use.
    pub(crate) columns: Vec<rockbound::schema::ColumnFamilyName>,
}

impl DbOptions {
    /// Setup [`rockbound::DB`] with default options
    pub fn default_setup_db_in_path(
        self,
        path: impl AsRef<std::path::Path>,
    ) -> anyhow::Result<rockbound::DB> {
        let config = rocks_db_config::gen_rocksdb_options(&Default::default(), false);
        let db_path = path.as_ref().join(self.path_suffix);
        rockbound::DB::open(db_path, self.name, self.columns, &config)
    }
}

pub(crate) fn ensure_version_is_correct(
    key: &SchemaKey,
    version: sov_rollup_interface::common::SlotNumber,
    found: Option<(
        (SchemaKey, sov_rollup_interface::common::SlotNumber),
        Option<SchemaValue>,
    )>,
) -> anyhow::Result<Option<SchemaValue>> {
    match found {
        Some(((found_key, found_version), value)) => {
            if &found_key == key {
                anyhow::ensure!(found_version <= version, "Bug! iterator isn't returning expected values. expected a version <= {version:} but found {found_version:}");
                Ok(value)
            } else {
                Ok(None)
            }
        }
        None => Ok(None),
    }
}
