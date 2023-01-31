use core::fmt::Debug;

use crate::serial::{Decode, Encode};

// NOTE: When naming traits, we use the naming convention below:
// *Trait IFF there's an associated type that would otherwise have the same name

pub trait BlockheaderTrait: PartialEq + Debug + CanonicalHash<Output = Self::Hash> {
    type Hash: Clone;
    fn prev_hash(&self) -> &Self::Hash;
}

pub trait CanonicalHash {
    type Output: AsRef<[u8]>;
    fn hash(&self) -> Self::Output;
}

pub trait BatchTrait: PartialEq + Debug + Encode + Decode {
    type Header: BlockheaderTrait;
    type Transaction: TransactionTrait;
    fn header(&self) -> &Self::Header;
    fn transactions(&self) -> &[Self::Transaction];
    fn take_transactions(self) -> Vec<Self::Transaction>;
}

pub trait TransactionTrait:
    PartialEq + Debug + CanonicalHash<Output = Self::Hash> + Encode + Decode
{
    type Hash: AsRef<[u8]>;
}

pub trait AddressTrait:
    PartialEq + Debug + Clone + AsRef<[u8]> + for<'a> TryFrom<&'a [u8]>
{
}

#[derive(Debug, PartialEq, Copy, Clone)]
pub struct InvalidAddress;
