#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub mod cache;

mod access;
mod utils;

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::{fmt, write};

pub use access::MergeError;

#[derive(Debug, Eq, PartialEq, Clone, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "std", derive(thiserror::Error))]
pub struct CacheKey {
    pub key: Arc<Vec<u8>>,
}

impl fmt::Display for CacheKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // TODO revisit how we display keys
        write!(f, "{:?}", self.key)
    }
}

#[derive(Debug, Eq, PartialEq, Clone)]
#[cfg_attr(feature = "std", derive(thiserror::Error))]
pub struct CacheValue {
    pub value: Arc<Vec<u8>>,
}

impl fmt::Display for CacheValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // TODO revisit how we display values
        write!(f, "{:?}", self.value)
    }
}
