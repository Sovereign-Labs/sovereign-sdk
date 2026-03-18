use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;

use crate::{Empty, Inner, MockZkGuest, Proof};

struct MockNetworkProof {
    proof_bytes: Vec<u8>,
    ready: bool,
}

/// A mock implementation of [`sov_rollup_interface::zk::ZkvmNetwork`].
///
/// Proofs can be controlled in two modes:
/// - **Manual** ([`MockZkvmNetwork::new`]): proofs stay pending until
///   [`MockZkvmNetwork::complete_proof`] is called with the returned handle.
/// - **Auto-complete** ([`MockZkvmNetwork::new_auto_complete`]): proofs are
///   immediately ready when submitted.
#[derive(Clone)]
pub struct MockZkvmNetwork {
    committed_data: VecDeque<Vec<u8>>,
    auto_complete: bool,
    proofs: Arc<Mutex<HashMap<u64, MockNetworkProof>>>,
    next_handle: Arc<AtomicU64>,
}

impl MockZkvmNetwork {
    /// Creates a new `MockZkvmNetwork` where proofs remain pending until
    /// [`Self::complete_proof`] is called.
    pub fn new(auto_complete: bool) -> Self {
        Self {
            committed_data: VecDeque::new(),
            auto_complete,
            proofs: Arc::new(Mutex::new(HashMap::new())),
            next_handle: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Marks a pending proof as ready, allowing [`ZkvmNetwork::poll`] to
    /// return its bytes.
    ///
    /// # Panics
    ///
    /// Panics if `handle` does not correspond to a submitted proof.
    pub fn complete_proof(&self, handle: u64) {
        let mut proofs = self.proofs.lock().unwrap();
        let proof = proofs
            .get_mut(&handle)
            .expect("complete_proof called with unknown handle");
        proof.ready = true;
    }

    /// Removes a proof from the internal map so that subsequent
    /// [`ZkvmNetwork::poll`] calls for this handle return an error.
    ///
    /// # Panics
    ///
    /// Panics if `handle` does not correspond to a submitted proof.
    pub fn fail_proof(&self, handle: u64) {
        let mut proofs = self.proofs.lock().unwrap();
        assert!(
            proofs.remove(&handle).is_some(),
            "fail_proof called with unknown handle"
        );
    }
}

impl Default for MockZkvmNetwork {
    fn default() -> Self {
        Self::new(false)
    }
}

impl sov_rollup_interface::zk::ZkvmNetwork for MockZkvmNetwork {
    type Guest = MockZkGuest;
    type ProofHandle = u64;

    fn add_hint<T: Serialize>(&mut self, item: &T) {
        let data = bincode::serialize(item).unwrap();
        self.committed_data.push_back(data);
    }

    async fn submit(&mut self) -> anyhow::Result<Self::ProofHandle> {
        let data = self.committed_data.pop_front().unwrap_or_default();
        let proof_bytes = bincode::serialize(&Proof::<Empty, Inner>::PublicData(Inner {
            is_valid: true,
            pub_data: data,
        }))?;

        let handle = self.next_handle.fetch_add(1, Ordering::Relaxed);
        let mut proofs = self.proofs.lock().unwrap();
        proofs.insert(
            handle,
            MockNetworkProof {
                proof_bytes,
                ready: self.auto_complete,
            },
        );
        Ok(handle)
    }

    async fn poll(&self, handle: &Self::ProofHandle) -> anyhow::Result<Option<Vec<u8>>> {
        let proofs = self.proofs.lock().unwrap();
        match proofs.get(handle) {
            Some(proof) if proof.ready => Ok(Some(proof.proof_bytes.clone())),
            Some(_) => Ok(None),
            None => anyhow::bail!("unknown proof handle: {handle}"),
        }
    }
}
