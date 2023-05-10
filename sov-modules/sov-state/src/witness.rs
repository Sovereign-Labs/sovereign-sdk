use borsh::{BorshDeserialize, BorshSerialize};
use jmt::storage::TreeReader;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::sync::atomic::AtomicUsize;

// TODO: Refactor witness trait so it only require Serialize / Deserialize
//   https://github.com/Sovereign-Labs/sovereign/issues/263
pub trait Witness: Default + Serialize {
    fn add_hint<T: BorshSerialize>(&self, hint: T);
    fn get_hint<T: BorshDeserialize>(&self) -> T;
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
            Some(val) => Ok(Some(jmt::storage::Node::deserialize_reader(&mut &val[..])?)),
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

#[derive(Default, Debug, Serialize, Deserialize)]
pub struct ArrayWitness {
    next_idx: AtomicUsize,
    hints: RefCell<Vec<Vec<u8>>>,
}

impl Witness for ArrayWitness {
    fn add_hint<T: BorshSerialize>(&self, hint: T) {
        self.hints.borrow_mut().push(hint.try_to_vec().unwrap())
    }

    fn get_hint<T: BorshDeserialize>(&self) -> T {
        let idx = self
            .next_idx
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        T::deserialize_reader(&mut std::io::Cursor::new(&self.hints.borrow()[idx]))
            .expect("Hint deserialization should never fail")
    }

    fn merge(&self, rhs: &Self) {
        let rhs_next_idx = rhs.next_idx.load(std::sync::atomic::Ordering::SeqCst);
        self.hints
            .borrow_mut()
            .extend(rhs.hints.borrow_mut().drain(rhs_next_idx..))
    }
}
