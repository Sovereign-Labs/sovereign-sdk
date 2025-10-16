//! Bytes prefix definition.

use core::{fmt, str};

use sov_rollup_interface::sov_universal_wallet::UniversalWallet;

/// A prefix prepended to each key before insertion and retrieval from the storage.
///
/// When interacting with state containers, you will usually use the same working set instance to
/// access them, as required by the module API. This also means that you might get key collisions,
/// so it becomes necessary to prepend a prefix to each key.
#[derive(
    Debug,
    PartialEq,
    Eq,
    Clone,
    Hash,
    PartialOrd,
    Ord,
    serde::Serialize,
    serde::Deserialize,
    borsh::BorshDeserialize,
    borsh::BorshSerialize,
    UniversalWallet,
    Copy,
)]
#[cfg_attr(
    feature = "arbitrary",
    derive(arbitrary::Arbitrary, proptest_derive::Arbitrary)
)]
pub struct Prefix {
    pub(crate) module: u8,
    pub(crate) item: u8,
}

impl Prefix {
    /// Returns a new [`Prefix`] with the item offset by the given amount.
    pub fn offset_by(self, offset: u8) -> Self {
        Prefix {
            module: self.module,
            item: self.item.checked_add(offset).expect("Overflow checking offset - this should be unreachable since we've already checked the offset"),
        }
    }
    /// Returns the module short id of the [`Prefix`].
    pub fn module(&self) -> u8 {
        self.module
    }
    /// Returns the item id of the [`Prefix`].
    pub fn item(&self) -> u8 {
        self.item
    }

    /// Returns a new [`Prefix`] with the given module and item discriminants.
    pub fn new(module: u8, item: u8) -> Self {
        Self { module, item }
    }
}

impl fmt::Display for Prefix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let buf = [self.module, self.item];
        write!(f, "0x{}", hex::encode(buf))
    }
}
