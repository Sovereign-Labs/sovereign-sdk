//! Defines types, traits, and helpers that are used by the core state-machine of the rollup.
//! Items in this module must be fully deterministic, since they are expected to be executed inside of zkVMs.
pub mod crypto;
pub mod da;
pub mod stf;
pub mod zk;

pub use bytes::{Buf, BufMut, Bytes, BytesMut};

#[cfg(feature = "mocks")]
pub mod mocks;
pub mod traits;
