#![cfg_attr(not(feature = "native"), no_std)]

mod avail;
#[cfg(feature = "native")]
pub mod service;
pub mod spec;
pub mod verifier;

// NOTE: Remove once dependency to the node is removed
pub use avail_subxt::build_client;
