use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tokio::sync::Notify;

use sov_rollup_interface::da::DaSpec;
use sov_rollup_interface::node::da::DaService;
use sov_stf_runner::processes::{
    ProofAggregationStatus, ProofProcessingStatus, ProverService, ProverServiceError,
    StateTransitionInfo, ZkProofManagerStatus,
};

struct ManualProofPostingGateState {
    blocked_proofs: usize,
    available_releases: usize,
    ready_proof_count: usize,
    is_open: bool,
}

pub(crate) struct ManualProofPostingSharedState {
    gate: Mutex<ManualProofPostingGateState>,
    notify: Notify,
}

impl ManualProofPostingSharedState {
    pub(crate) fn new() -> Self {
        Self {
            gate: Mutex::new(ManualProofPostingGateState {
                blocked_proofs: 0,
                available_releases: 0,
                ready_proof_count: 0,
                is_open: false,
            }),
            notify: Notify::new(),
        }
    }

    fn ready_proof_count(&self) -> usize {
        self.gate
            .lock()
            .expect("manual proof posting gate lock poisoned")
            .ready_proof_count
    }

    async fn wait_for_ready_proof_count(&self, target: usize) {
        loop {
            let notified = self.notify.notified();
            if self.ready_proof_count() >= target {
                return;
            }
            notified.await;
        }
    }

    async fn wait_until_release_is_allowed(&self) {
        let should_block = {
            let mut gate = self
                .gate
                .lock()
                .expect("manual proof posting gate lock poisoned");
            gate.ready_proof_count = gate.ready_proof_count.saturating_add(1);

            if gate.is_open {
                false
            } else {
                gate.blocked_proofs = gate.blocked_proofs.saturating_add(1);
                true
            }
        };
        self.notify.notify_waiters();

        if !should_block {
            return;
        }

        {
            loop {
                let notified = self.notify.notified();
                {
                    let mut gate = self
                        .gate
                        .lock()
                        .expect("manual proof posting gate lock poisoned");

                    if gate.is_open {
                        gate.blocked_proofs = gate.blocked_proofs.saturating_sub(1);
                        return;
                    }

                    if gate.available_releases > 0 {
                        gate.available_releases -= 1;
                        gate.blocked_proofs = gate.blocked_proofs.saturating_sub(1);
                        return;
                    }
                }
                notified.await;
            }
        }
    }

    fn release_next_proof(&self) {
        let mut gate = self
            .gate
            .lock()
            .expect("manual proof posting gate lock poisoned");
        gate.available_releases += 1;
        drop(gate);
        self.notify.notify_waiters();
    }

    fn open(&self) {
        let mut gate = self
            .gate
            .lock()
            .expect("manual proof posting gate lock poisoned");
        gate.is_open = true;
        drop(gate);
        self.notify.notify_waiters();
    }
}

/// Control handle for manually releasing aggregate proofs to DA in tests.
pub struct ManualProofPostingControl {
    shared_state: Arc<ManualProofPostingSharedState>,
    proof_manager_status: Arc<ZkProofManagerStatus>,
}

impl ManualProofPostingControl {
    pub(crate) fn new(
        shared_state: Arc<ManualProofPostingSharedState>,
        proof_manager_status: Arc<ZkProofManagerStatus>,
    ) -> Self {
        Self {
            shared_state,
            proof_manager_status,
        }
    }

    /// Releases the next blocked aggregate proof for publication.
    pub fn release_next_proof(&self) {
        self.shared_state.release_next_proof();
    }

    /// Returns how many aggregate proofs have reached the posting gate.
    pub fn ready_proof_count(&self) -> usize {
        self.shared_state.ready_proof_count()
    }

    /// Waits until at least `target` aggregate proofs have reached the posting gate.
    pub async fn wait_for_ready_proof_count(&self, target: usize) {
        self.shared_state.wait_for_ready_proof_count(target).await;
    }

    /// Permanently opens the gate so all current and future proofs flow through.
    pub fn open(&self) {
        self.shared_state.open();
    }

    /// Returns how many more blocks are needed before the next aggregate proof is ready.
    pub fn blocks_until_next_aggregate_proof(&self) -> usize {
        self.proof_manager_status
            .blocks_until_next_aggregate_proof()
    }
}

impl Drop for ManualProofPostingControl {
    fn drop(&mut self) {
        if let Ok(mut gate) = self.shared_state.gate.lock() {
            gate.is_open = true;
        }
        self.shared_state.notify.notify_waiters();
    }
}

/// A [`ProverService`] wrapper which blocks aggregate-proof publication until released by a test.
pub struct ManualProofPostingProverService<Ps: ProverService> {
    inner: Arc<Ps>,
    shared_state: Arc<ManualProofPostingSharedState>,
}

impl<Ps: ProverService> Clone for ManualProofPostingProverService<Ps> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            shared_state: self.shared_state.clone(),
        }
    }
}

impl<Ps: ProverService> ManualProofPostingProverService<Ps> {
    pub(crate) fn new(inner: Ps, shared_state: Arc<ManualProofPostingSharedState>) -> Self {
        Self {
            inner: Arc::new(inner),
            shared_state,
        }
    }
}

#[async_trait]
impl<Ps: ProverService> ProverService for ManualProofPostingProverService<Ps> {
    type StateRoot = Ps::StateRoot;
    type Witness = Ps::Witness;
    type DaService = Ps::DaService;
    type Verifier = Ps::Verifier;

    async fn prove(
        &self,
        state_transition_info: StateTransitionInfo<
            Self::StateRoot,
            Self::Witness,
            <Self::DaService as DaService>::Spec,
        >,
    ) -> Result<
        ProofProcessingStatus<Self::StateRoot, Self::Witness, <Self::DaService as DaService>::Spec>,
        ProverServiceError,
    > {
        self.inner.prove(state_transition_info).await
    }

    async fn create_aggregated_proof(
        &self,
        block_header_hashes: &[<<Self::DaService as DaService>::Spec as DaSpec>::SlotHash],
        genesis_state_root: &Self::StateRoot,
    ) -> anyhow::Result<ProofAggregationStatus> {
        match self
            .inner
            .create_aggregated_proof(block_header_hashes, genesis_state_root)
            .await?
        {
            ProofAggregationStatus::Success(aggregated_proof) => {
                self.shared_state.wait_until_release_is_allowed().await;
                Ok(ProofAggregationStatus::Success(aggregated_proof))
            }
            other => Ok(other),
        }
    }
}
