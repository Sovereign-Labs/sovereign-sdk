use thiserror::Error;

pub mod cache;

mod access;
mod utils;

use std::fmt::Display;
use std::sync::Arc;

pub use access::MergeError;

#[derive(Error, Debug, Eq, PartialEq, Clone, Hash, PartialOrd, Ord)]
pub struct CacheKey {
    pub key: Arc<Vec<u8>>,
}

impl Display for CacheKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // TODO revisit how we display keys
        write!(f, "{:?}", self.key)
    }
}

#[derive(Error, Debug, Eq, PartialEq, Clone)]
pub struct CacheValue {
    pub value: Arc<Vec<u8>>,
}

impl Display for CacheValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // TODO revisit how we display values
        write!(f, "{:?}", self.value)
    }
}
