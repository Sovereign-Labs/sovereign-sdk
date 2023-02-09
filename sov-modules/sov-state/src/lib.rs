mod jmt_storage;
mod map;
pub mod storage;
mod utils;
mod value;

pub use jmt_storage::JmtStorage;
pub use map::StateMap;
pub use storage::Storage;
use utils::AlignedVec;
pub use value::StateValue;

// A prefix prepended to each key before insertion and retrieval from the storage.
// All the collection types in this crate are backed by the same storage instance, this means that insertions of the same key
// to two different `StorageMaps` would collide with each other. We solve it by instantiating every collection type with a unique
// prefix that is prepended to each key.
#[derive(Debug, PartialEq, Eq)]
pub struct Prefix {
    prefix: AlignedVec,
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
