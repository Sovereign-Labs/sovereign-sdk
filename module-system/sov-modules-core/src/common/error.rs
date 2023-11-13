//! Module error definitions.

use alloc::string::String;

use crate::storage::CacheValue;

/// Representation of a signature verification error.
#[derive(Debug)]
#[cfg_attr(feature = "std", derive(thiserror::Error))]
pub enum SigVerificationError {
    /// The signature is invalid for the provided public key.
    #[cfg_attr(feature = "std", error("Bad signature {0}"))]
    BadSignature(String),
}

#[cfg(not(feature = "std"))]
impl core::fmt::Display for SigVerificationError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        <SigVerificationError as core::fmt::Debug>::fmt(self, f)
    }
}

#[cfg(all(not(feature = "std"), feature = "sync"))]
impl From<SigVerificationError> for anyhow::Error {
    fn from(err: SigVerificationError) -> anyhow::Error {
        anyhow::Error::msg(err)
    }
}

/// A bech32 address parse error.
#[derive(Debug)]
#[cfg_attr(feature = "std", derive(thiserror::Error))]
pub enum Bech32ParseError {
    /// Bech32 decoding error represented via [bech32::Error].
    #[cfg_attr(feature = "std", error("Bech32 error: {0}"))]
    Bech32(#[cfg_attr(feature = "std", from)] bech32::Error),
    /// The provided "Human-Readable Part" is invalid.
    #[cfg_attr(feature = "std", error("Wrong HRP: {0}"))]
    WrongHPR(String),
}

#[cfg(not(feature = "std"))]
impl core::fmt::Display for Bech32ParseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        <Bech32ParseError as core::fmt::Debug>::fmt(self, f)
    }
}

#[cfg(all(not(feature = "std"), feature = "sync"))]
impl From<Bech32ParseError> for anyhow::Error {
    fn from(err: Bech32ParseError) -> anyhow::Error {
        anyhow::Error::msg(err)
    }
}

#[cfg(not(feature = "std"))]
impl From<bech32::Error> for Bech32ParseError {
    fn from(err: bech32::Error) -> Bech32ParseError {
        Bech32ParseError::Bech32(err)
    }
}

/// An error when merging two cache values.
#[derive(Debug, PartialEq, Eq)]
#[cfg_attr(feature = "std", derive(thiserror::Error))]
pub enum MergeError {
    /// Consecutive reads error.
    #[cfg_attr(
        feature = "std",
        error("consecutive reads are inconsistent: left read: {left:?}, right read: {right:?}")
    )]
    ReadThenRead {
        /// Left-read associated cache value.
        left: Option<CacheValue>,
        /// Right-read associated cache value.
        right: Option<CacheValue>,
    },
    /// A read operation is inconsistent with the previous write operation.
    #[cfg_attr(
        feature = "std",
        error("the read: {read:?} is in inconsistent with the previous write: {write:?}")
    )]
    WriteThenRead {
        /// The associated write operation.
        write: Option<CacheValue>,
        /// The associated read operation.
        read: Option<CacheValue>,
    },
}

#[cfg(not(feature = "std"))]
impl core::fmt::Display for MergeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        <MergeError as core::fmt::Debug>::fmt(self, f)
    }
}

#[cfg(all(not(feature = "std"), feature = "sync"))]
impl From<MergeError> for anyhow::Error {
    fn from(err: MergeError) -> anyhow::Error {
        anyhow::Error::msg(err)
    }
}

/// An error when reading from the cache.
#[derive(Debug, Eq, PartialEq)]
#[cfg_attr(feature = "std", derive(thiserror::Error))]
pub enum ReadError {
    /// The value returned from the cache is not expected.
    #[cfg_attr(
        feature = "std",
        error("inconsistent read, expected: {expected:?}, found: {found:?}")
    )]
    InconsistentRead {
        /// Expected value.
        expected: Option<CacheValue>,
        /// Found value.
        found: Option<CacheValue>,
    },
}

#[cfg(not(feature = "std"))]
impl core::fmt::Display for ReadError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        <ReadError as core::fmt::Debug>::fmt(self, f)
    }
}

#[cfg(all(not(feature = "std"), feature = "sync"))]
impl From<ReadError> for anyhow::Error {
    fn from(err: ReadError) -> anyhow::Error {
        anyhow::Error::msg(err)
    }
}

/// General error type in the Module System.
#[derive(Debug)]
#[cfg_attr(feature = "std", derive(thiserror::Error))]
pub enum ModuleError {
    /// Custom error thrown by a module.
    #[cfg_attr(feature = "std", error(transparent))]
    ModuleError(#[cfg_attr(feature = "std", from)] anyhow::Error),
}

#[cfg(not(feature = "std"))]
impl core::fmt::Display for ModuleError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        <ModuleError as core::fmt::Debug>::fmt(self, f)
    }
}

#[cfg(all(not(feature = "std"), feature = "sync"))]
impl From<ModuleError> for anyhow::Error {
    fn from(err: ModuleError) -> anyhow::Error {
        anyhow::Error::msg(err)
    }
}

#[cfg(not(feature = "std"))]
impl From<anyhow::Error> for ModuleError {
    fn from(err: anyhow::Error) -> ModuleError {
        ModuleError::ModuleError(err)
    }
}
