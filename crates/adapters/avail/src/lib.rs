#[cfg(feature = "native")]
mod avail;
#[cfg(feature = "native")]
pub mod service;
pub mod spec;
pub mod verifier;

// NOTE: Remove once dependency to the node is removed
#[cfg(feature = "native")]
pub use avail_subxt::build_client;
