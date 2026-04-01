use std::collections::VecDeque;
use std::time::Duration;

use sov_rollup_interface::da::DaSpec;
use sov_rollup_interface::node::da::DaService;
use sov_rollup_interface::zk::aggregated_proof::SerializedAggregatedProof;

use crate::processes::{
    ProofAggregationStatus, ProofProcessingStatus, ProverService, StateTransitionInfo,
};

/// A [`VecDeque`] which is guaranteed to contain at least one item at all
/// times.
struct NonEmptyVecDeque<T: Default>(VecDeque<T>);

impl<T: Default> NonEmptyVecDeque<T> {
    /// Creates a new queue with a single default element.
    pub fn new_with_default() -> Self {
        let mut deque = VecDeque::new();
        deque.push_back(Default::default());
        Self(deque)
    }

    pub fn front_mut(&mut self) -> &mut T {
        self.0
            .front_mut()
            .expect("NonEmptyVecDeque may not be empty")
    }

    /// Pops from the front of the queue, pushing `Default::default` if the
    /// queue would be emptied after the operation. To avoid adding the default
    /// element, the caller should check the queue length and `push_back` a value
    /// of their own if necessary.
    pub fn pop_front_with_default(&mut self) -> T {
        if self.0.len() == 1 {
            self.0.push_back(Default::default());
        }
        self.0
            .pop_front()
            .expect("NonEmptyVecDeque may not be empty")
    }

    pub fn back(&self) -> &T {
        self.0.back().expect("NonEmptyVecDeque may not be empty")
    }

    pub fn back_mut(&mut self) -> &mut T {
        self.0
            .back_mut()
            .expect("NonEmptyVecDeque may not be empty")
    }

    pub fn push_back(&mut self, value: T) {
        self.0.push_back(value);
    }
}

/// The current status of a block proof.
pub(crate) enum BlockProofStatus<W> {
    /// The proof has not yet been accepted by the prover service
    Waiting(W),
    /// The proof has been submitted to the prover service
    Submitted,
}

/// Metadata for an aggregated proof that has not yet been created.
pub(crate) struct AggregateProofMetadata<Ps: ProverService> {
    /// The proof info for each individual block covered by this proof
    block_proof_info: Vec<BlockProofInfo<Ps>>,
    /// Set to true if and only if all subproofs have been submitted to the prover service.
    is_ready: bool,
    /// The estimated size of the aggregated proof, including any public data needed to verify it.
    estimated_proof_size: u64,
}

impl<Ps: ProverService> Default for AggregateProofMetadata<Ps> {
    fn default() -> Self {
        Self {
            block_proof_info: Vec::new(),
            is_ready: false,
            estimated_proof_size: 0,
        }
    }
}

impl<Ps: ProverService> AggregateProofMetadata<Ps> {
    pub fn push(&mut self, block: BlockProofInfo<Ps>) {
        self.estimated_proof_size += block.public_data_size;
        if !matches!(block.status, BlockProofStatus::Submitted) {
            self.is_ready = false;
        }
        self.block_proof_info.push(block);
    }

    pub async fn prove_any_unproven_blocks(&mut self, prover_service: &Ps) {
        if self.is_ready {
            return;
        }
        for proof in self.block_proof_info.iter_mut() {
            let mut prev_status = BlockProofStatus::Submitted;
            std::mem::swap(&mut prev_status, &mut proof.status);
            if let BlockProofStatus::Waiting(mut witness) = prev_status {
                // TODO: Add backoff on proof submission attempts
                //  <https://github.com/Sovereign-Labs/sovereign-sdk-wip/issues/446>
                loop {
                    let status = prover_service
                        .prove(witness)
                        .await
                        .expect("The proof submission should succeed");

                    // Stop the runner loop until prover is ready.
                    match status {
                        ProofProcessingStatus::ProvingInProgress => break,
                        ProofProcessingStatus::Busy(data) => {
                            witness = data;
                            tokio::time::sleep(Duration::from_millis(100)).await;
                        }
                    }
                }
            }
        }
        self.is_ready = true;
    }

    pub async fn prove(
        mut self,
        prover_service: &Ps,
        genesis_state_root: &Ps::StateRoot,
    ) -> Result<SerializedAggregatedProof, (Self, anyhow::Error)> {
        self.prove_any_unproven_blocks(prover_service).await;
        let agg_proof_hashes: Vec<_> = self
            .block_proof_info
            .iter()
            .map(|info| info.hash.clone())
            .collect();

        loop {
            let status = prover_service
                .create_aggregated_proof(agg_proof_hashes.as_slice(), genesis_state_root)
                .await;

            match status {
                Ok(ProofAggregationStatus::Success(agg_proof)) => {
                    return Ok(agg_proof);
                }
                // TODO(https://github.com/Sovereign-Labs/sovereign-sdk/issues/1185): Add timeout handling.
                Ok(ProofAggregationStatus::ProofGenerationInProgress) => {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                Err(e) => return Err((self, e)),
            }
        }
    }
}

/// A block which needs to be proven
pub(crate) struct BlockProofInfo<Ps: ProverService> {
    /// The current status of the proof for this block
    pub status: BlockProofStatus<ProverStateTransitionInfo<Ps>>,
    /// The hash of this block
    pub hash: <<Ps::DaService as DaService>::Spec as DaSpec>::SlotHash,

    /// The size of any public data needed to verify a proof of this block, in bytes
    pub public_data_size: u64,
}

// The type alias bound is required here because we access associated types... but rustc still complains 🤷‍♂️
#[allow(type_alias_bounds)]
type ProverStateTransitionInfo<Ps: ProverService> =
    StateTransitionInfo<Ps::StateRoot, Ps::Witness, <Ps::DaService as DaService>::Spec>;

/// Contains an ordered list of lists of blocks to be incorporated in aggregate proofs. Each sublist
/// will be grouped into a single aggregate proof.
pub struct UnAggregatedProofList<Ps: ProverService> {
    /// The list of groups of blocks. We maintain the invariants
    /// that this queue is never empty, and that new blocks are always pushed
    /// to the end of the last sublist.
    proof_queue: NonEmptyVecDeque<AggregateProofMetadata<Ps>>,
    /// An estimate of the total size (in bytes) of all the aggregate which
    /// are currently planned
    running_proof_size_estimate: u64,
}

impl<Ps: ProverService> UnAggregatedProofList<Ps> {
    pub fn new() -> Self {
        Self {
            proof_queue: NonEmptyVecDeque::new_with_default(),
            running_proof_size_estimate: 0,
        }
    }

    /// Returns the number of blocks scheduled to be proven
    /// in the current aggregated proof.
    pub fn current_proof_jump(&self) -> usize {
        self.proof_queue.back().block_proof_info.len()
    }

    /// Creates a new aggregate proof metadata instance at the tail of the queue, causing
    /// subsequent `append` calls to modify the new metadata.  
    pub fn close_newest_proof(&mut self) {
        self.proof_queue
            .push_back(AggregateProofMetadata::default());
    }

    /// Adds a block proof to the newest metadata instance
    pub fn append(&mut self, block: BlockProofInfo<Ps>) {
        self.running_proof_size_estimate += block.public_data_size;
        self.proof_queue.back_mut().push(block);
    }

    /// Takes the metadata for the oldest aggregated proof in the queue
    pub fn oldest_mut(&mut self) -> &mut AggregateProofMetadata<Ps> {
        self.proof_queue.front_mut()
    }

    /// Takes the metadata for the oldest aggregated proof in the queue
    pub fn take_oldest(&mut self) -> AggregateProofMetadata<Ps> {
        self.proof_queue.pop_front_with_default()
    }
}
