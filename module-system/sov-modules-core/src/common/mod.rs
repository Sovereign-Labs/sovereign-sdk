//! Common types shared between state and modules

mod address;
mod bytes;
mod error;
mod gas;
mod key;
mod witness;

pub use address::*;
pub use bytes::*;
pub use error::*;
pub use gas::*;
#[cfg(feature = "std")]
pub use jmt::Version;
pub use key::*;
pub use witness::*;

/// The version of the JellyfishMerkleTree state.
#[cfg(not(feature = "std"))]
pub type Version = u64;
