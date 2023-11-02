#![cfg_attr(not(feature = "std"), no_std)]
#![deny(missing_docs)]
#![doc = include_str!("../README.md")]

extern crate alloc;

#[cfg(all(feature = "sync", not(target_has_atomic = "ptr")))]
compile_error!("The `sync` feature is not supported on this architecture.");

pub mod common;
pub mod module;
pub mod storage;

pub use common::*;
pub use module::*;
pub use storage::*;
