use std::{cell::RefCell, sync::atomic::AtomicUsize};

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

use super::traits::Witness;

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
