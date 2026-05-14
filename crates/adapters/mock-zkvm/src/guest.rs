use std::sync::Mutex;

use serde::Serialize;

use crate::MockZkVerifier;

/// A mock implementing the Guest.
#[derive(Default)]
pub struct MockZkGuest {
    hint: Mutex<Option<Vec<u8>>>,
    committed: Mutex<Option<Vec<u8>>>,
}

impl MockZkGuest {
    /// Construct a guest seeded with a single bincode-serialized hint.
    pub fn with_hint(hint: Vec<u8>) -> Self {
        Self {
            hint: Mutex::new(Some(hint)),
            committed: Mutex::new(None),
        }
    }

    /// Drain the bytes committed by [`Self::commit`]. Returns `None` if
    /// `commit` was never called.
    pub fn take_committed_bytes(&self) -> Option<Vec<u8>> {
        self.committed
            .lock()
            .expect("MockZkGuest committed mutex poisoned")
            .take()
    }
}

impl sov_rollup_interface::zk::ZkvmGuest for MockZkGuest {
    type Verifier = MockZkVerifier;
    fn read_from_host<T: serde::de::DeserializeOwned>(&self) -> T {
        let bytes = self
            .hint
            .lock()
            .expect("MockZkGuest hint mutex poisoned")
            .take()
            .expect("MockZkGuest has no hint to read; call `with_hint` first");
        bincode::deserialize(&bytes).expect("failed to deserialize MockZkGuest hint")
    }

    fn commit<T: Serialize>(&self, item: &T) {
        let bytes = bincode::serialize(item).expect("failed to serialize MockZkGuest commit");
        *self
            .committed
            .lock()
            .expect("MockZkGuest committed mutex poisoned") = Some(bytes);
    }
}
