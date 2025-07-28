use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

/// A key-value pair representing a change to the rollup state.
#[derive(Debug, Clone, PartialEq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "arbitrary"), derive(proptest_derive::Arbitrary))]
pub struct StoredEvent {
    key: EventKey,
    value: EventValue,
    tx_hash: [u8; 32],
}

impl StoredEvent {
    /// Create a new event with the given key, value, and transaction hash.
    pub fn new(key: &[u8], value: &[u8], tx_hash: [u8; 32]) -> Self {
        Self {
            key: EventKey(key.to_vec()),
            value: EventValue(value.to_vec()),
            tx_hash,
        }
    }

    /// Get the event key.
    pub fn key(&self) -> &EventKey {
        &self.key
    }

    /// Get the event value
    pub fn value(&self) -> &EventValue {
        &self.value
    }

    /// Get the transaction hash that emitted this event.
    pub fn tx_hash(&self) -> &[u8; 32] {
        &self.tx_hash
    }
}

/// The key of an event. This is a wrapper around a `Vec<u8>`.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    BorshSerialize,
    BorshDeserialize,
    Serialize,
    Deserialize,
)]
#[cfg_attr(any(test, feature = "arbitrary"), derive(proptest_derive::Arbitrary))]
pub struct EventKey(Vec<u8>);

impl EventKey {
    /// Create a new event serialized from Typed Event.
    pub fn new(value: &[u8]) -> Self {
        Self(value.to_vec())
    }

    /// Return the inner bytes of the event key.
    pub fn inner(&self) -> &Vec<u8> {
        &self.0
    }
}

/// The value of an event. This is a wrapper around a `Vec<u8>`.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "arbitrary"), derive(proptest_derive::Arbitrary))]
pub struct EventValue(Vec<u8>);

impl EventValue {
    /// Return the inner bytes of the event value.
    /// Return the inner bytes of the event key.
    pub fn inner(&self) -> &Vec<u8> {
        &self.0
    }
}
