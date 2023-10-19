//! This crate defines the core traits and types used by all Sovereign SDK rollups.
//! It specifies the interfaces which allow the same "business logic" to run on different
//! DA layers and be proven with different zkVMS, all while retaining compatibility
//! with the same basic full node implementation.

#![cfg_attr(not(feature = "std"), no_std)]
#![deny(missing_docs)]

mod state_machine;
pub use state_machine::*;

mod node;

pub use digest;
pub use node::*;

/// A facade for the `std` crate.
pub mod maybestd {
    pub use borsh::maybestd::{borrow, boxed, collections, format, io, rc, string, vec};

    /// A facade for the `sync` std module.
    pub mod sync {
        #[cfg(feature = "std")]
        pub use std::sync::Mutex;

        pub use borsh::maybestd::sync::*;
        #[cfg(not(feature = "std"))]
        pub use spin::Mutex;
    }
}
