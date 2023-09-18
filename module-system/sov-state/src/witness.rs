use std::sync::atomic::AtomicUsize;
use std::sync::Mutex;

use borsh::{BorshDeserialize, BorshSerialize};
use jmt::storage::TreeReader;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// A witness is a value produced during native execution that is then used by
/// the zkVM circuit to produce proofs.
///
/// Witnesses are typically used to abstract away storage access from inside the
/// zkVM. For every read operation performed by the native code, a hint can be
/// added and the zkVM circuit can then read the same hint. Hints are replayed
/// to [`Witness::get_hint`] in the same order
/// they were added via [`Witness::add_hint`].
// TODO: Refactor witness trait so it only require Serialize / Deserialize
//   https://github.com/Sovereign-Labs/sovereign-sdk/issues/263
pub trait Witness: Default + Serialize + DeserializeOwned {
    /// Adds a serializable "hint" to the witness value, which can be later
    /// read by the zkVM circuit.
    ///
    /// This method **SHOULD** only be called from the native execution
    /// environment.
    fn add_hint<T: BorshSerialize>(&self, hint: T);

    /// Retrieves a "hint" from the witness value.
    fn get_hint<T: BorshDeserialize>(&self) -> T;

    /// Adds all hints from `rhs` to `self`.
    fn merge(&self, rhs: &Self);
}

/// A wrapper around a [`Witness`] that implements [`TreeReader`].
#[derive(Debug)]
pub struct TreeWitnessReader<'a, T: Witness>(&'a T);

impl<'a, T: Witness> TreeWitnessReader<'a, T> {
    /// Wraps the given witness.
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

/// A [`Vec`]-based implementation of [`Witness`] with no special logic.
///
/// # Example
///
/// ```
/// use sov_state::{ArrayWitness, Witness};
///
/// let witness = ArrayWitness::default();
///
/// witness.add_hint(1u64);
/// witness.add_hint(2u64);
///
/// assert_eq!(witness.get_hint::<u64>(), 1u64);
/// assert_eq!(witness.get_hint::<u64>(), 2u64);
/// ```
#[derive(Default, Debug, Serialize, Deserialize)]
pub struct ArrayWitness {
    next_idx: AtomicUsize,
    hints: Mutex<Vec<Vec<u8>>>,
}

impl Witness for ArrayWitness {
    fn add_hint<T: BorshSerialize>(&self, hint: T) {
        self.hints.lock().unwrap().push(hint.try_to_vec().unwrap())
    }

    fn get_hint<T: BorshDeserialize>(&self) -> T {
        let idx = self
            .next_idx
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let hints_lock = self.hints.lock().unwrap();
        T::deserialize_reader(&mut std::io::Cursor::new(&hints_lock[idx]))
            .expect("Hint deserialization should never fail")
    }

    fn merge(&self, rhs: &Self) {
        let rhs_next_idx = rhs.next_idx.load(std::sync::atomic::Ordering::SeqCst);
        let mut lhs_hints_lock = self.hints.lock().unwrap();
        let mut rhs_hints_lock = rhs.hints.lock().unwrap();
        lhs_hints_lock.extend(rhs_hints_lock.drain(rhs_next_idx..))
    }
}
