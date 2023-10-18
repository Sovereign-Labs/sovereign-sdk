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
pub use db::{SchemaBatch, DB};
mod iterator;
#[cfg(feature = "std")]
mod metrics;
pub mod schema;

#[cfg(feature = "std")]
pub use iterator::SchemaIterator;
pub use iterator::SeekKeyEncoder;
#[cfg(feature = "std")]
pub use rocksdb::DEFAULT_COLUMN_FAMILY_NAME;
pub use schema::Schema;

use sov_rollup_interface::maybestd::io;

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
        <CodecError as core::fmt::Debug>::fmt(self, f)
    }
}

#[cfg(not(feature = "std"))]
impl From<CodecError> for anyhow::Error {
    fn from(e: CodecError) -> Self {
        anyhow::Error::msg(e)
    }
}

#[cfg(not(feature = "std"))]
impl From<io::Error> for CodecError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}
