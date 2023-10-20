// SPDX-License-Identifier: Apache-2.0
// Adapted from aptos-core/schemadb

#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! This library implements a schematized DB on top of [RocksDB](https://rocksdb.org/). It makes
//! sure all data passed in and out are structured according to predefined schemas and prevents
//! access to raw keys and values. This library also enforces a set of specific DB options,
//! like custom comparators and schema-to-column-family mapping.
//!
//! It requires that different kinds of key-value pairs be stored in separate column
//! families.  To use this library to store a kind of key-value pairs, the user needs to use the
//! [`define_schema!`] macro to define the schema name, the types of key and value, and name of the
//! column family.

#[cfg(feature = "std")]
mod db;
#[cfg(feature = "std")]
pub use db::DB;
mod iterator;
#[cfg(feature = "std")]
mod metrics;
pub mod schema;

#[cfg(feature = "std")]
pub use iterator::SchemaIterator;
pub use iterator::SeekKeyEncoder;
use sov_rollup_interface::maybestd::collections::HashMap;
use sov_rollup_interface::maybestd::io;
use sov_rollup_interface::maybestd::sync::Mutex;
use sov_rollup_interface::maybestd::vec::Vec;

pub use crate::schema::Schema;
use crate::schema::{ColumnFamilyName, KeyCodec, ValueCodec};

#[derive(Debug)]
#[cfg_attr(not(feature = "std"), allow(dead_code))]
enum WriteOp {
    Value { key: Vec<u8>, value: Vec<u8> },
    Deletion { key: Vec<u8> },
}

/// [`SchemaBatch`] holds a collection of updates that can be applied to a DB
/// ([`Schema`]) atomically. The updates will be applied in the order in which
/// they are added to the [`SchemaBatch`].
#[derive(Debug, Default)]
pub struct SchemaBatch {
    rows: Mutex<HashMap<ColumnFamilyName, Vec<WriteOp>>>,
}

impl SchemaBatch {
    /// Creates an empty batch.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds an insert/update operation to the batch.
    pub fn put<S: Schema>(
        &self,
        key: &impl KeyCodec<S>,
        value: &impl ValueCodec<S>,
    ) -> anyhow::Result<()> {
        #[cfg(feature = "std")]
        let _timer = metrics::SCHEMADB_BATCH_PUT_LATENCY_SECONDS
            .with_label_values(&["unknown"])
            .start_timer();

        let key = key.encode_key()?;
        let value = value.encode_value()?;

        #[cfg(feature = "std")]
        self.rows
            .lock()
            .expect("Lock must not be poisoned")
            .entry(S::COLUMN_FAMILY_NAME)
            .or_default()
            .push(WriteOp::Value { key, value });

        #[cfg(not(feature = "std"))]
        self.rows
            .lock()
            .entry(S::COLUMN_FAMILY_NAME)
            .or_default()
            .push(WriteOp::Value { key, value });

        Ok(())
    }

    /// Adds a delete operation to the batch.
    pub fn delete<S: Schema>(&self, key: &impl KeyCodec<S>) -> anyhow::Result<()> {
        let key = key.encode_key()?;

        #[cfg(feature = "std")]
        self.rows
            .lock()
            .expect("Lock must not be poisoned")
            .entry(S::COLUMN_FAMILY_NAME)
            .or_default()
            .push(WriteOp::Deletion { key });

        #[cfg(not(feature = "std"))]
        self.rows
            .lock()
            .entry(S::COLUMN_FAMILY_NAME)
            .or_default()
            .push(WriteOp::Deletion { key });

        Ok(())
    }
}

/// An error that occurred during (de)serialization of a [`Schema`]'s keys or
/// values.
#[derive(Debug)]
#[cfg_attr(feature = "std", derive(thiserror::Error))]
pub enum CodecError {
    /// Unable to deserialize a key because it has a different length than
    /// expected.
    #[cfg_attr(
        feature = "std",
        error("Invalid key length. Expected {expected:}, got {got:}")
    )]
    #[allow(missing_docs)] // The fields' names are self-explanatory.
    InvalidKeyLength { expected: usize, got: usize },
    /// Some other error occurred when (de)serializing a key or value. Inspect
    /// the inner [`anyhow::Error`] for more details.
    #[cfg_attr(feature = "std", error(transparent))]
    Wrapped(#[cfg_attr(feature = "std", from)] anyhow::Error),
    /// I/O error.
    #[cfg_attr(feature = "std", error(transparent))]
    Io(#[cfg_attr(feature = "std", from)] io::Error),
}

#[cfg(not(feature = "std"))]
impl core::fmt::Display for CodecError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[cfg(not(feature = "std"))]
impl From<CodecError> for anyhow::Error {
    fn from(e: CodecError) -> Self {
        anyhow::Error::msg(e)
    }
}

#[cfg(not(feature = "std"))]
impl From<anyhow::Error> for CodecError {
    fn from(e: anyhow::Error) -> Self {
        CodecError::Wrapped(e)
    }
}

#[cfg(not(feature = "std"))]
impl From<io::Error> for CodecError {
    fn from(e: io::Error) -> Self {
        CodecError::Io(e)
    }
}
