//! Defines types, traits, and helpers that are used by the core state-machine of the rollup.
//! Items in this module must be fully deterministic, since they are expected to be executed inside of zkVMs.
pub mod crypto;
pub mod da;
pub mod stf;
pub mod zk;

#[cfg(feature = "std")]
pub use bytes::{Buf, BufMut, Bytes, BytesMut};
use serde::de::DeserializeOwned;
use serde::Serialize;

pub mod optimistic;
pub mod storage;

/// A marker trait for general addresses.
pub trait BasicAddress:
    Eq
    + PartialEq
    + core::fmt::Debug
    + core::fmt::Display
    + Send
    + Sync
    + Clone
    + core::hash::Hash
    + AsRef<[u8]>
    + for<'a> TryFrom<&'a [u8], Error = anyhow::Error>
    + core::str::FromStr
    + Serialize
    + DeserializeOwned
    + 'static
{
}

/// An address used inside rollup
pub trait RollupAddress: BasicAddress + From<[u8; 32]> {}
