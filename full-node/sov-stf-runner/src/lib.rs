#![deny(missing_docs)]
#![doc = include_str!("../README.md")]

#[cfg(feature = "native")]
mod batch_builder;
#[cfg(feature = "native")]
mod config;

#[cfg(feature = "native")]
pub use config::RpcConfig;
#[cfg(feature = "native")]
mod ledger_rpc;
#[cfg(feature = "native")]
mod runner;
#[cfg(feature = "native")]
pub use batch_builder::FiFoStrictBatchBuilder;
#[cfg(feature = "native")]
pub use config::{from_toml_path, RollupConfig, RunnerConfig, StorageConfig};
#[cfg(feature = "native")]
pub use ledger_rpc::get_ledger_rpc;
#[cfg(feature = "native")]
pub use runner::*;
