use alloc::string::String;

use crate::cache::CacheValue;

#[derive(Debug)]
#[cfg_attr(feature = "std", derive(thiserror::Error))]
pub enum SigVerificationError {
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

#[derive(Debug)]
#[cfg_attr(feature = "std", derive(thiserror::Error))]
pub enum Bech32ParseError {
    #[cfg_attr(feature = "std", error("Bech32 error: {0}"))]
    Bech32(#[cfg_attr(feature = "std", from)] bech32::Error),
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

#[derive(Debug, PartialEq, Eq)]
#[cfg_attr(feature = "std", derive(thiserror::Error))]
pub enum MergeError {
    #[cfg_attr(
        feature = "std",
        error("consecutive reads are inconsistent: left read: {left:?}, right read: {right:?}")
    )]
    ReadThenRead {
        left: Option<CacheValue>,
        right: Option<CacheValue>,
    },
    #[cfg_attr(
        feature = "std",
        error("the read: {read:?} is in inconsistent with the previous write: {write:?}")
    )]
    WriteThenRead {
        write: Option<CacheValue>,
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

#[derive(Debug, Eq, PartialEq)]
#[cfg_attr(feature = "std", derive(thiserror::Error))]
pub enum ReadError {
    #[cfg_attr(
        feature = "std",
        error("inconsistent read, expected: {expected:?}, found: {found:?}")
    )]
    InconsistentRead {
        expected: Option<CacheValue>,
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
