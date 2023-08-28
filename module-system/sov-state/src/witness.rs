use std::sync::atomic::AtomicUsize;
use std::sync::Mutex;

use borsh::{BorshDeserialize, BorshSerialize};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

// TODO: Refactor witness trait so it only require Serialize / Deserialize
//   https://github.com/Sovereign-Labs/sovereign-sdk/issues/263
pub trait Witness: Default + Serialize + DeserializeOwned {
    fn add_hint<T: BorshSerialize>(&self, hint: T);
    fn get_hint<T: BorshDeserialize>(&self) -> T;
    fn merge(&self, rhs: &Self);
}

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
