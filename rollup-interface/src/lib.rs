//! This crate defines the core traits and types used by all Sovereign SDK rollups.
//! It specifies the interfaces which allow the same "business logic" to run on different
//! DA layers and be proven with different zkVMS, all while retaining compatibility
//! with the same basic full node implementation.

#![cfg_attr(not(feature = "std"), no_std)]
#![deny(missing_docs)]

extern crate alloc;

mod state_machine;
pub use state_machine::*;

mod node;

pub use node::*;
pub use {anyhow, digest};

/// A facade for the `std` crate.
pub mod maybestd {
    #[cfg(not(target_has_atomic = "ptr"))]
    pub use alloc::rc::Rc as RefCount;
    // sync will be available only when the target supports atomic operations
    #[cfg(target_has_atomic = "ptr")]
    pub use alloc::sync;
    #[cfg(target_has_atomic = "ptr")]
    pub use alloc::sync::Arc as RefCount;

    pub use borsh::maybestd::{borrow, boxed, collections, format, io, string, vec};
}
