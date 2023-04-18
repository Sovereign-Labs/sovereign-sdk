use core::fmt::Debug;

use jmt::storage::TreeReader;

use crate::serial::{Decode, Encode};

// NOTE: When naming traits, we use the naming convention below:
// *Trait IFF there's an associated type that would otherwise have the same name

pub trait BlockHeaderTrait: PartialEq + Debug + CanonicalHash<Output = Self::Hash> {
    type Hash: Clone;
    fn prev_hash(&self) -> &Self::Hash;
}

pub trait CanonicalHash {
    type Output: AsRef<[u8]>;
    fn hash(&self) -> Self::Output;
}

pub trait BatchTrait: PartialEq + Debug + Encode + Decode + Clone {
    type Transaction: TransactionTrait;
    fn transactions(&self) -> &[Self::Transaction];
    fn take_transactions(self) -> Vec<Self::Transaction>;
}

pub trait TransactionTrait:
    PartialEq + Debug + CanonicalHash<Output = Self::Hash> + Encode + Decode
{
    type Hash: AsRef<[u8]>;
}

pub trait AddressTrait:
    PartialEq + Debug + Clone + AsRef<[u8]> + for<'a> TryFrom<&'a [u8], Error = anyhow::Error> + Eq
{
}

#[derive(Debug, PartialEq, Copy, Clone)]
pub struct InvalidAddress;

pub trait Witness: Default {
    fn add_hint<T: Encode>(&self, hint: T);
    fn get_hint<T: Decode>(&self) -> T;
    fn merge(&self, rhs: &Self);
}

#[derive(Debug)]
pub struct TreeWitnessReader<'a, T: Witness>(&'a T);

impl<'a, T: Witness> TreeWitnessReader<'a, T> {
    pub fn new(witness: &'a T) -> Self {
        Self(witness)
    }
}

impl<'a, T: Witness> TreeReader for TreeWitnessReader<'a, T> {
    fn get_node_option(
        &self,
        _node_key: &jmt::storage::NodeKey,
    ) -> anyhow::Result<Option<jmt::storage::Node>> {
        let serialized_node_opt: Option<Vec<u8>> = self.0.get_hint();
        match serialized_node_opt {
            Some(val) => Ok(Some(jmt::storage::Node::decode(&val)?)),
            None => Ok(None),
        }
    }

    fn get_value_option(
        &self,
        _max_version: jmt::Version,
        _key_hash: jmt::KeyHash,
    ) -> anyhow::Result<Option<jmt::OwnedValue>> {
        Ok(self.0.get_hint())
    }

    fn get_rightmost_leaf(
        &self,
    ) -> anyhow::Result<Option<(jmt::storage::NodeKey, jmt::storage::LeafNode)>> {
        unimplemented!()
    }
}
