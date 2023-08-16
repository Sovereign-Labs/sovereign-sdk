//! Defines types, traits, and helpers that are used by the core state-machine of the rollup.
//! Items in this module must be fully deterministic, since they are expected to be executed inside of zkVMs.
pub mod crypto;
pub mod da;
pub mod stf;
pub mod zk;

use borsh::{BorshDeserialize, BorshSerialize};
pub use bytes::{Buf, BufMut, Bytes, BytesMut};
use serde::de::DeserializeOwned;
use serde::Serialize;

#[cfg(feature = "mocks")]
pub mod mocks;

pub mod optimistic;

/// A marker trait for addresses.
pub trait AddressTrait:
    PartialEq
    + core::fmt::Debug
    + Clone
    + AsRef<[u8]>
    + for<'a> TryFrom<&'a [u8], Error = anyhow::Error>
    + Eq
    + Serialize
    + DeserializeOwned
    + From<[u8; 32]>
    + Send
    + Sync
    + core::fmt::Display
    + std::hash::Hash
    + 'static
{
}
