#![cfg_attr(not(feature = "std"), no_std)]
#![deny(missing_docs)]
#![doc = include_str!("../README.md")]

extern crate alloc;

#[cfg(all(feature = "sync", not(target_has_atomic = "ptr")))]
compile_error!("The `sync` feature is not supported on this architecture.");

pub mod address;
pub mod bytes;
pub mod cache;
pub mod codec;
pub mod dispatch;
pub mod error;
pub mod gas;
pub mod key;
pub mod module;
pub mod scratchpad;
pub mod spec;
pub mod storage;
pub mod version;
pub mod witness;

pub use address::*;
pub use bytes::*;
pub use cache::*;
pub use codec::*;
pub use dispatch::*;
pub use error::*;
pub use gas::*;
pub use key::*;
pub use module::*;
pub use scratchpad::*;
pub use spec::*;
pub use storage::*;
pub use version::*;
pub use witness::*;
