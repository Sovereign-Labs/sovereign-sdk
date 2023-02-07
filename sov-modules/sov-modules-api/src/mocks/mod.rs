use crate::Context;

#[derive(borsh::BorshDeserialize, PartialEq, Eq)]
pub struct MockPublicKey {
    pub_key: Vec<u8>,
}

impl MockPublicKey {
    pub fn new(pub_key: Vec<u8>) -> Self {
        Self { pub_key }
    }
}

#[derive(borsh::BorshDeserialize)]
pub struct MockSignature {
    _sig: Vec<u8>,
}

impl MockSignature {
    pub fn new(sig: Vec<u8>) -> Self {
        Self { _sig: sig }
    }
}

#[derive(Clone)]
pub struct MockStorage {}

impl sov_state::Storage for MockStorage {
    fn get(
        &mut self,
        _key: sov_state::storage::StorageKey,
        _version: u64,
    ) -> Option<sov_state::storage::StorageValue> {
        todo!()
    }

    fn set(
        &mut self,
        _key: sov_state::storage::StorageKey,
        _version: u64,
        _value: sov_state::storage::StorageValue,
    ) {
        todo!()
    }

    fn delete(&mut self, _key: sov_state::storage::StorageKey, _version: u64) {
        todo!()
    }
}

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
