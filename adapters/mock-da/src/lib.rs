#![deny(missing_docs)]

//! Mock implementation of DaService, DaSpec and DaVerifier for testing.

#[cfg(feature = "native")]
mod service;
mod types;
mod validity_condition;
/// Contains DaSpec and DaVerifier
pub mod verifier;

#[cfg(feature = "native")]
pub use service::*;
pub use types::*;
pub use validity_condition::*;
pub use verifier::MockDaSpec;
