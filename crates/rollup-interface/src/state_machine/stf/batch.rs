//! Transaction batch types and utilities.
//!
//! This module defines the core transaction types used throughout the Sovereign SDK,
//! including fully authenticated transactions ready for DA layer submission.

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

use crate::Bytes;

/// `FullyBakedTx` represents a serialized signed rollup transaction that has been encoded with
/// authentication information and is ready to be placed on the DA layer.
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
    pub data: Bytes,
    /// Sequencer-provided metadata for each transaction (e.g., timestamps).
    /// This data is NOT signed by users but is added by the sequencer.
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
}
