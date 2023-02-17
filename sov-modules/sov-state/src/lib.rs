mod backend;
mod internal_cache;
mod jmt_storage;
mod map;
pub mod storage;
mod utils;
mod value;
mod zk_storage;

#[cfg(test)]
mod storage_test;

pub use jmt_storage::JmtStorage;
pub use map::StateMap;
use std::{fmt::Display, str};
pub use storage::Storage;
use utils::AlignedVec;
pub use zk_storage::ZkStorage;

pub use value::StateValue;

// A prefix prepended to each key before insertion and retrieval from the storage.
// All the collection types in this crate are backed by the same storage instance, this means that insertions of the same key
// to two different `StorageMaps` would collide with each other. We solve it by instantiating every collection type with a unique
// prefix that is prepended to each key.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Prefix {
    prefix: AlignedVec,
}

impl Display for Prefix {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let buf = self.prefix.as_ref();
        write!(f, "{:?}", str::from_utf8(buf).unwrap())
    }
}

impl Prefix {
    pub fn new(prefix: Vec<u8>) -> Self {
        Self {
            prefix: AlignedVec::new(prefix),
        }
    }

    pub fn as_aligned_vec(&self) -> &AlignedVec {
        &self.prefix
    }

    pub fn len(&self) -> usize {
        self.prefix.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
