#![deny(missing_docs)]
#![doc = include_str!("../README.md")]
#[cfg(feature = "native")]
mod builder;

pub mod da;
mod hooks;
#[cfg(feature = "native")]
pub mod rollup;

pub mod zkvm;

#[cfg(feature = "native")]
mod rpc;

pub mod runtime;
