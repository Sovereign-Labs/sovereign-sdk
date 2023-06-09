use core::fmt::{Debug, Display};

use serde::de::DeserializeOwned;
use serde::Serialize;

// NOTE: When naming traits, we use the naming convention below:
// *Trait IFF there's an associated type that would otherwise have the same name

pub trait BlockHeaderTrait: PartialEq + Debug + CanonicalHash<Output = Self::Hash> + Clone {
    type Hash: Clone;
    fn prev_hash(&self) -> Self::Hash;
}

pub trait CanonicalHash {
    type Output: AsRef<[u8]>;
    fn hash(&self) -> Self::Output;
}

pub trait BatchTrait: PartialEq + Debug + Serialize + DeserializeOwned + Clone {
    type Transaction: TransactionTrait;
    fn transactions(&self) -> &[Self::Transaction];
    fn take_transactions(self) -> Vec<Self::Transaction>;
}

pub trait TransactionTrait: PartialEq + Debug + Serialize + DeserializeOwned {
    type Hash: AsRef<[u8]>;
}

pub trait AddressTrait:
    PartialEq
    + Debug
    + Clone
    + AsRef<[u8]>
    + for<'a> TryFrom<&'a [u8], Error = anyhow::Error>
    + Eq
    + Serialize
    + DeserializeOwned
    + From<[u8; 32]>
    + Send
    + Sync
    + Display
{
}
