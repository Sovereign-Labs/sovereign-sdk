//! Transaction batch types and utilities.
//!
//! This module defines the core transaction types used throughout the Sovereign SDK,
//! including fully authenticated transactions ready for DA layer submission.

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use serde_with::serde_as;

use crate::Bytes;

/// `FullyBakedTx` represents a serialized signed rollup transaction that has been encoded with
/// authentication information and is ready to be placed on the DA layer.
#[serde_as]
#[derive(
    PartialEq,
    Eq,
    Clone,
    BorshDeserialize,
    BorshSerialize,
    Serialize,
    Deserialize,
    Default,
    derive_more::AsRef,
)]
pub struct FullyBakedTx {
    /// Serialized transaction.
    #[as_ref(forward)]
    #[serde_as(as = "serde_with::base64::Base64")]
    pub data: Bytes,
    /// Sequencer-provided metadata for each transaction (e.g., timestamps).
    /// This data is NOT signed by users but is added by the sequencer.
    #[serde_as(as = "Option<serde_with::base64::Base64>")]
    pub sequencing_data: Option<Bytes>,
}

impl std::fmt::Debug for FullyBakedTx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FullyBakedTx")
            .field("data", &hex::encode(&self.data))
            .finish()
    }
}

impl FullyBakedTx {
    /// Construct a `FullyBakedTx` containing the given data
    #[must_use]
    pub fn new(data: Vec<u8>) -> Self {
        Self {
            data: Bytes::from_owner(data),
            sequencing_data: None,
        }
    }

    /// Sets sequencing metadata
    pub fn set_sequencing_metadata(&mut self, metadata: &impl BorshSerialize) {
        self.sequencing_data = Some(borsh::to_vec(metadata).unwrap().into());
    }

    /// Returns the total serialized length of the transaction, including both
    /// the transaction data and sequencing metadata (if present).
    /// This is the length that will be sent on the DA layer.
    #[must_use]
    pub fn len(&self) -> usize {
        borsh::to_vec(self)
            .expect("Serialization to vec is infallible")
            .len()
    }

    /// Returns true if the transaction has no data
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.data.is_empty() && self.sequencing_data.is_none()
    }
}

/// `RawTx` represents a serialized signed rollup transaction. A `RawTx` needs to be encoded
/// with authentication information before being placed on the DA layer.
#[derive(
    PartialEq,
    Eq,
    Clone,
    BorshDeserialize,
    BorshSerialize,
    Serialize,
    Deserialize,
    derive_more::AsRef,
)]
pub struct RawTx {
    /// Serialized transaction.
    #[as_ref(forward)]
    pub data: Vec<u8>,
}

impl std::fmt::Debug for RawTx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RawTx")
            .field("data", &hex::encode(&self.data))
            .finish()
    }
}

impl RawTx {
    /// Construct a `RawTx` containing the given data
    #[must_use]
    pub fn new(data: Vec<u8>) -> Self {
        Self { data }
    }

    /// Returns the length of the serialized transaction data
    #[must_use]
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Returns true if the transaction has no data
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}
