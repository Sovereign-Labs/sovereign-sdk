use std::fmt::Debug;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use super::StoredEvent;
use crate::TxHash;

/// Ignored transactions consume gas but do not otherwise impact the state of the rollup.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound = "T: TxReceiptContents")]
pub struct IgnoredTransactionReceipt<T: TxReceiptContents> {
    /// The receipt.
    pub ignored: T::Ignored,
}

/// A receipt for a single transaction. These receipts are stored in the rollup's database
/// and may be queried via RPC. Receipts are generic over a type `R` which the rollup can use to
/// store additional data, such as the status code of the transaction or the amount of gas used.
#[derive(Debug, Clone, Serialize, Deserialize)]
/// A receipt showing the result of a transaction
#[serde(bound = "T: TxReceiptContents")]
pub struct TransactionReceipt<T: TxReceiptContents> {
    /// The canonical hash of this transaction
    pub tx_hash: TxHash,
    /// The canonically serialized body of the transaction, if it should be persisted
    /// in the database
    pub body_to_save: Option<Vec<u8>>,
    /// The events output by this transaction
    pub events: Vec<StoredEvent>,
    /// Any additional structured data to be saved in the database and served over RPC
    /// For example, this might contain a status code.
    pub receipt: TxEffect<T>,
}

/// The outcome of a transaction.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(any(test, feature = "arbitrary"), derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "snake_case")]
pub enum TxEffect<T: TxReceiptContents> {
    /// The transaction was skipped.
    Skipped(T::Skipped),
    /// The transaction was reverted during execution.
    Reverted(T::Reverted),
    /// The transaction was processed successfully.
    Successful(T::Successful),
}

/// A (typically zero-sized) struct which marks the contents of a [`TxEffect`].
// We require a bunch of bounds on the marker struct to work around issues with rust's type inference
// even though they aren't strictly needed.
pub trait TxReceiptContents:
    Debug + Clone + PartialEq + Serialize + DeserializeOwned + Send + Sync + 'static
{
    /// The receipt contents for a skipped transaction.
    type Skipped: Debug
        + Clone
        + PartialEq
        + Eq
        + Serialize
        + DeserializeOwned
        + Send
        + Sync
        + 'static;
    /// The receipt contents for a reverted transaction.
    type Reverted: Debug
        + Clone
        + PartialEq
        + Eq
        + Serialize
        + DeserializeOwned
        + Send
        + Sync
        + 'static;
    /// The receipt contents for a successful transaction.
    type Successful: Debug
        + Clone
        + PartialEq
        + Eq
        + Serialize
        + DeserializeOwned
        + Send
        + Sync
        + 'static;

    /// The receipt contents for an ignored transaction.
    type Ignored: Debug
        + Clone
        + PartialEq
        + Eq
        + Serialize
        + DeserializeOwned
        + Send
        + Sync
        + 'static;
}

impl TxReceiptContents for () {
    type Skipped = ();
    type Reverted = ();
    type Successful = ();
    type Ignored = ();
}

impl<T: TxReceiptContents> TxEffect<T> {
    /// Returns true if and only if the effect is the [`TxEffect::Successful`] variant.
    pub fn is_successful(&self) -> bool {
        matches!(self, TxEffect::Successful(_))
    }

    /// Returns true if and only if the effect is the [`TxEffect::Reverted`] variant.
    pub fn is_reverted(&self) -> bool {
        matches!(self, TxEffect::Reverted(_))
    }

    /// Returns true if and only if the effect is the [`TxEffect::Skipped`] variant.
    pub fn is_skipped(&self) -> bool {
        matches!(self, TxEffect::Skipped(_))
    }
}
