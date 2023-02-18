use crate::{
    storage::{StorageKey, StorageValue},
    Prefix, Storage,
};
use sovereign_sdk::serial::{Decode, Encode};
use std::marker::PhantomData;

#[derive(Debug)]
pub(crate) struct Backend<K, V, S> {
    _phantom: (PhantomData<K>, PhantomData<V>),
    storage: S,
    /// Every instance of the `Backend` contains a unique prefix.
    /// The prefix is prepended to each key before insertion and retrieval from the storage.
    prefix: Prefix,
}

impl<K: Encode, V: Encode + Decode, S: Storage> Backend<K, V, S> {
    pub(crate) fn new(storage: S, prefix: Prefix) -> Self {
        Self {
            _phantom: (PhantomData, PhantomData),
            storage,
            prefix,
        }
    }

    pub(crate) fn prefix(&self) -> &Prefix {
        &self.prefix
    }

    pub(crate) fn set_value(&mut self, storage_key: StorageKey, value: V) {
        let storage_value = StorageValue::new(value);
        self.storage.set(storage_key, storage_value);
    }

    pub(crate) fn get_value(&self, storage_key: StorageKey) -> Option<V> {
        let storage_value = self.storage.get(storage_key)?;

        // It is ok to panic here. Deserialization problem means that something is terribly wrong.
        Some(
            V::decode(&mut storage_value.value())
                .unwrap_or_else(|e| panic!("Unable to deserialize storage value {e:?}")),
        )
    }
}
