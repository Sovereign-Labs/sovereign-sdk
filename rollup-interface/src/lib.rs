//! This crate defines the core traits and types used by all Sovereign SDK rollups.
//! It specifies the interfaces which allow the same "business logic" to run on different
//! DA layers and be proven with different zkVMS, all while retaining compatibility
//! with the same basic full node implementation.
#![deny(missing_docs)]
mod state_machine;
pub use state_machine::*;

mod node;

pub use borsh::maybestd;
pub use node::*;
pub use digest;
