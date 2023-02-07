use crate::Prefix;
use std::marker::PhantomData;

// A container that maps keys to values.
#[derive(Debug)]
pub struct StateMap<K, V, S> {
    _phantom: (PhantomData<K>, PhantomData<V>),
    _storage: S,
    // Every instance of the `StateMap` contains a unique prefix.
    // The prefix is prepended to each key before insertion and retrieval from the storage.
    prefix: Prefix,
}

impl<K, V, S> StateMap<K, V, S> {
    pub fn new(_storage: S, prefix: Prefix) -> Self {
        Self {
            _phantom: (PhantomData, PhantomData),
            _storage,
            prefix,
        }
    }

    // Inserts a key-value pair into the map.
    pub fn set(&mut self, _k: K, _v: V) {}

    // Returns the value corresponding to the key or None if key is absent in the StateMap.
    pub fn get(&mut self, _k: K) -> Option<V> {
        todo!()
    }

    pub fn prefix(&self) -> &Prefix {
        &self.prefix
    }
}
