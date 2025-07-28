//! Configuration options for [`Storage`](crate::storage::Storage) types.

use std::path::PathBuf;

/// Configuration options for [`ProverStorage`](crate::ProverStorage)
/// initialization.
#[derive(serde::Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Path to folder where storage files will be stored.
    pub path: PathBuf,
}
