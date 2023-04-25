use std::{cell::RefCell, sync::atomic::AtomicUsize};

use super::traits::Witness;

#[derive(Default)]
pub struct ArrayWitness {
    next_idx: AtomicUsize,
    hints: RefCell<Vec<Vec<u8>>>,
}

impl Witness for ArrayWitness {
    fn add_hint<T: crate::serial::Encode>(&self, hint: T) {
        self.hints.borrow_mut().push(hint.encode_to_vec())
    }

    fn get_hint<T: crate::serial::Decode>(&self) -> T {
        let idx = self
            .next_idx
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        T::decode_from_slice(&self.hints.borrow()[idx]).unwrap()
    }

    fn merge(&self, rhs: &Self) {
        let rhs_next_idx = rhs.next_idx.load(std::sync::atomic::Ordering::SeqCst);
        self.hints
            .borrow_mut()
            .extend(rhs.hints.borrow_mut().drain(rhs_next_idx..))
    }
}
