//! Celestia DA adapter that posts and reads rollup blobs through the Fibre protocol.
//!
//! Blob data lives off-chain with Fibre validators; the Celestia namespace only carries
//! v2 shares referencing it (blob version + commitment).
#[cfg(feature = "native")]
mod config;
#[cfg(feature = "native")]
mod da_service;
mod spec;

#[cfg(feature = "native")]
pub use config::CelestiaFibreConfig;
#[cfg(feature = "native")]
pub use da_service::{CelestiaFibreDaService, FilteredFibreBlock};
pub use spec::{CelestiaFibreSpec, CelestiaFibreVerifier, FibreBlob};
