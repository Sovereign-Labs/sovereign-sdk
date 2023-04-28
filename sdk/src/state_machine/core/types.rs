use std::{cell::RefCell, sync::atomic::AtomicUsize};

use borsh::maybestd;

use crate::serial::{Decode, DecodeBorrowed, Encode};

use super::traits::Witness;

#[derive(Default, Debug)]
pub struct ArrayWitness {
    next_idx: AtomicUsize,
    hints: RefCell<Vec<Vec<u8>>>,
}

impl Encode for ArrayWitness {
    fn encode(&self, target: &mut impl std::io::Write) {
        let hints = self.hints.borrow();
        let next_idx = self.next_idx.load(std::sync::atomic::Ordering::SeqCst);
        hints[next_idx..].as_ref().encode(target);
    }
}

impl<'de> DecodeBorrowed<'de> for ArrayWitness {
    type Error = maybestd::io::Error;

    fn decode_from_slice(target: &'de [u8]) -> Result<Self, Self::Error> {
        Self::decode(&mut std::io::Cursor::new(target))
    }
}

impl Decode for ArrayWitness {
    type Error = maybestd::io::Error;

    fn decode<R: std::io::Read>(target: &mut R) -> Result<Self, <Self as Decode>::Error> {
        let hints = Vec::<Vec<u8>>::decode(target)?;
        Ok(ArrayWitness {
            next_idx: AtomicUsize::new(0),
            hints: RefCell::new(hints),
        })
    }
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
