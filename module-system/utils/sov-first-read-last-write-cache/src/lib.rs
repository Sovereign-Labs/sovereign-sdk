pub mod cache;

mod access;
mod utils;

use core::fmt::Display;

pub use access::MergeError;
use sov_rollup_interface::maybestd::sync::Arc;

#[derive(Debug, Eq, PartialEq, Clone, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "std", derive(thiserror::Error))]
pub struct CacheKey {
    pub key: Arc<Vec<u8>>,
}

impl Display for CacheKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // TODO revisit how we display keys
        write!(f, "{:?}", self.key)
    }
}

#[derive(Debug, Eq, PartialEq, Clone)]
#[cfg_attr(feature = "std", derive(thiserror::Error))]
pub struct CacheValue {
    pub value: Arc<Vec<u8>>,
}

impl Display for CacheValue {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // TODO revisit how we display values
        write!(f, "{:?}", self.value)
    }
}
