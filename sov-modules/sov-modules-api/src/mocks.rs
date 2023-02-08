use crate::Context;
use sov_state::storage::{StorageKey, StorageValue};
use std::{cell::RefCell, collections::HashMap, rc::Rc, sync::Arc};

/// Mock for Context::PublicKey, useful for testing.
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, PartialEq, Eq)]
pub struct MockPublicKey {
    pub_key: Vec<u8>,
}

impl MockPublicKey {
    pub fn new(pub_key: Vec<u8>) -> Self {
        Self { pub_key }
    }
}

/// Mock for Context::Signature, useful for testing.
#[derive(borsh::BorshDeserialize, PartialEq, Eq)]
pub struct MockSignature {
    sig: Vec<u8>,
}

impl MockSignature {
    pub fn new(sig: Vec<u8>) -> Self {
        Self { sig }
    }
}

type Storage = Rc<RefCell<HashMap<Arc<Vec<u8>>, Arc<Vec<u8>>>>>;

/// Mock for Context::Storage, useful for testing.
// TODO: as soon as we have JMT storage implemented, we should remove this mock and use a real db even in tests.
// see https://github.com/Sovereign-Labs/sovereign/issues/40
#[derive(Clone, Default, Debug)]
pub struct MockStorage {
    storage: Storage,
}

impl sov_state::Storage for MockStorage {
    fn get(&self, key: StorageKey) -> Option<StorageValue> {
        self.storage
            .borrow()
            .get(key.as_ref())
            .map(|v| StorageValue { value: v.clone() })
    }

    fn set(&mut self, key: StorageKey, value: StorageValue) {
        self.storage.borrow_mut().insert(key.key(), value.value);
    }

    fn delete(&mut self, key: StorageKey) {
        self.storage.borrow_mut().remove(&key.key());
    }
}

/// Mock for Context, useful for testing.
pub struct MockContext {
    sender: MockPublicKey,
}

impl Context for MockContext {
    type Storage = MockStorage;

    type Signature = MockSignature;

    type PublicKey = MockPublicKey;

    fn sender(&self) -> &Self::PublicKey {
        &self.sender
    }
}
