use std::sync::{Arc, Mutex};

use crate::notifier::NotificationManager;
use crate::{MockCodeCommitment, MockProof, MockZkGuest};
use serde::{Deserialize, Serialize};
use sov_rollup_interface::common::SlotNumber;
use sov_rollup_interface::da::DaSpec;
use sov_rollup_interface::zk::aggregated_proof::AggregatedProofPublicData;
use sov_rollup_interface::zk::aggregated_proof::{
    BlockProof, OuterZkvmHost, SerializedAggregatedProof,
};
use sov_rollup_interface::zk::SerializedZkProof;

/// A mock implementing the zkVM trait.
#[derive(Clone)]
pub struct MockZkvmHost {
    notification_manager: NotificationManager,
    wait_for_proof: bool,
    /// Most recently produced aggregated proof. Shared across clones so that
    /// continuity assertions in [`OuterZkvmHost::run_proof_aggregation`] hold
    /// across the whole prover service.
    previous_aggregated_proof: Arc<Mutex<Option<SerializedAggregatedProof>>>,
}

/// Mirrors the prefix of [`AggregatedProofPublicData`](sov_rollup_interface::zk::aggregated_proof::AggregatedProofPublicData)
/// so we can read the slot bounds of a previous aggregated proof without
/// requiring `DeserializeOwned` bounds on the generic `Address`/`Root` types.
#[derive(Deserialize)]
struct AggregatedProofSlotBounds {
    // Read so bincode advances to `final_slot_number`; not used directly.
    #[allow(dead_code)]
    initial_slot_number: SlotNumber,
    final_slot_number: SlotNumber,
}

impl MockZkvmHost {
    /// Creates a new MockZkvm.
    pub fn new() -> Self {
        Self {
            wait_for_proof: true,
            notification_manager: Default::default(),
            previous_aggregated_proof: Arc::new(Mutex::new(None)),
        }
    }

    /// Creates a new MockZkvm, the `ZkvmHost::add_hint_and_run` will return immediately.
    pub fn new_non_blocking() -> Self {
        Self::new_non_blocking_with_previous_proof(None)
    }

    /// Like [`Self::new_non_blocking`], but seeded with the latest aggregated
    /// proof previously persisted in the ledger DB so that continuity
    /// assertions in [`OuterZkvmHost::run_proof_aggregation`] survive a node
    /// restart.
    pub fn new_non_blocking_with_previous_proof(
        previous_aggregated_proof: Option<SerializedAggregatedProof>,
    ) -> Self {
        Self {
            wait_for_proof: false,
            notification_manager: Default::default(),
            previous_aggregated_proof: Arc::new(Mutex::new(previous_aggregated_proof)),
        }
    }

    /// Simulates zk proof generation.
    pub fn make_proof(&self) {
        // We notify the worker thread.
        self.notification_manager.notify();
    }

    /// Create a proof for MockZkvm
    pub fn create_serialized_proof<T: Serialize>(
        is_valid: bool,
        transition: T,
    ) -> SerializedZkProof {
        let data = bincode::serialize(&transition).unwrap();
        let raw_proof = bincode::serialize(&MockProof {
            is_valid,
            pub_data: data,
        })
        .unwrap();
        SerializedZkProof { raw_proof }
    }

    fn add_hint_and_run_inner<T: Serialize>(&self, item: &T) -> anyhow::Result<Vec<u8>> {
        let pub_data = bincode::serialize(item)?;
        if self.wait_for_proof {
            self.notification_manager.wait();
        }
        Ok(bincode::serialize(&MockProof {
            is_valid: true,
            pub_data,
        })?)
    }
}

impl Default for MockZkvmHost {
    fn default() -> Self {
        Self::new()
    }
}

impl sov_rollup_interface::zk::ZkvmHost for MockZkvmHost {
    type Guest = MockZkGuest;

    fn code_commitment(&self) -> anyhow::Result<<<Self::Guest as sov_rollup_interface::zk::ZkvmGuest>::Verifier as sov_rollup_interface::zk::ZkVerifier>::CodeCommitment>{
        Ok(MockCodeCommitment::default())
    }

    fn add_hint_deferred_and_run<T: Serialize>(
        &mut self,
        item: &T,
        _agg_proofs: Vec<SerializedAggregatedProof>,
    ) -> anyhow::Result<SerializedZkProof> {
        self.add_hint_and_run_inner(item)
            .map(|raw_proof| SerializedZkProof { raw_proof })
    }
}

impl OuterZkvmHost for MockZkvmHost {
    fn run_proof_aggregation<Address: Serialize + Clone, Da: DaSpec, Root: Serialize + Clone>(
        &self,
        genesis_state_root: Root,
        headers_with_block_proofs: Vec<(Da::BlockHeader, BlockProof<Address, Da, Root>)>,
    ) -> anyhow::Result<SerializedAggregatedProof> {
        let block_proofs_data = headers_with_block_proofs
            .iter()
            .map(|(_, bp)| bp)
            .collect::<Vec<_>>();

        let public_data = AggregatedProofPublicData::from_block_proofs(
            block_proofs_data.as_slice(),
            genesis_state_root,
        );

        let mut previous = self
            .previous_aggregated_proof
            .lock()
            .expect("previous_aggregated_proof mutex was poisoned");

        if let Some(previous_proof) = previous.as_ref() {
            let envelope: MockProof = bincode::deserialize(&previous_proof.raw_aggregated_proof)
                .expect("previous aggregated proof envelope must be a MockProof");
            // `bincode::deserialize_from` reads only the bytes needed for the
            // declared fields, so we can extract the slot bounds without knowing
            // the concrete `Address`/`Root` types of the previous proof.
            let prev_bounds: AggregatedProofSlotBounds =
                bincode::deserialize_from(envelope.pub_data.as_slice())
                    .expect("previous aggregated proof public data must start with slot bounds");

            assert_eq!(
                public_data.initial_slot_number,
                prev_bounds.final_slot_number.next(),
                "Aggregated proof continuity violated: new aggregation starts at slot {} but previous aggregation ended at slot {}",
                public_data.initial_slot_number,
                prev_bounds.final_slot_number,
            );
        } else {
            assert_eq!(public_data.initial_slot_number, SlotNumber::ONE);
        }

        let serialized = self
            .add_hint_and_run_inner(&public_data)
            .map(|raw_aggregated_proof| SerializedAggregatedProof {
                raw_aggregated_proof,
            })?;

        *previous = Some(serialized.clone());
        Ok(serialized)
    }
}
