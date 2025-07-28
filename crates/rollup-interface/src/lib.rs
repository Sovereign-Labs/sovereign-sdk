//! This crate defines the core traits and types used by all Sovereign SDK rollups.
//! It specifies the interfaces which allow the same "business logic" to run on different
//! DA layers and be proven with different zkVMS, all while retaining compatibility
//! with the same basic full node implementation.

#![deny(missing_docs)]

pub mod common;
mod state_machine;
pub use state_machine::*;
#[cfg(feature = "native")]
pub mod node;

pub use sov_universal_wallet;

/// Useful third-party crate re-exports.
pub mod reexports {
    pub use {anyhow, digest, schemars};
}
