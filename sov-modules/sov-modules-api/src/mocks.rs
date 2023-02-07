use crate::Context;
use sov_state::storage::{StorageKey, StorageValue};
use std::{collections::HashMap, sync::Arc};

/// Mock for Context::PublicKey, useful for testing.
#[derive(borsh::BorshDeserialize, PartialEq, Eq)]
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

/// Mock for Context::Storage, useful for testing.
// TODO: as soon as we have JMT storage implemented, we should remove this mock and use a real db even in tests.
// see https://github.com/Sovereign-Labs/sovereign/issues/40
#[derive(Clone, Default)]
pub struct MockStorage {
    storage: HashMap<Arc<Vec<u8>>, Arc<Vec<u8>>>,
}

impl sov_state::Storage for MockStorage {
    fn get(&mut self, key: StorageKey, _version: u64) -> Option<StorageValue> {
        self.storage
            .get(&key.key)
            .map(|v| StorageValue { value: v.clone() })
    }

    fn set(&mut self, key: StorageKey, _version: u64, value: StorageValue) {
        self.storage.insert(key.key, value.value);
    }

    fn delete(&mut self, key: StorageKey, _version: u64) {
        self.storage.remove(&key.key);
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
