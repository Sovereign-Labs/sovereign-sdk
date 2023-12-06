#![cfg_attr(not(feature = "std"), no_std)]
#![deny(missing_docs)]
#![doc = include_str!("../README.md")]

extern crate alloc;
extern crate core;

#[cfg(all(feature = "sync", not(target_has_atomic = "ptr")))]
compile_error!("The `sync` feature is not supported on this architecture.");

pub mod common;
pub mod module;
pub mod runtime;
pub mod storage;

pub use common::*;
pub use module::*;
pub use runtime::*;
pub use storage::*;
