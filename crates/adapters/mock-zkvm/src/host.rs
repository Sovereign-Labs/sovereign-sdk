use std::fmt::Debug;
use std::sync::{Arc, Mutex};

use crate::notifier::NotificationManager;
use crate::{MockCodeCommitment, MockProof, MockZkGuest};
use serde::de::DeserializeOwned;
use serde::Serialize;
use sov_rollup_interface::common::SlotNumber;
use sov_rollup_interface::da::{BlockHeaderTrait, DaSpec};
use sov_rollup_interface::zk::aggregated_proof::{
    AggregatedProofPublicData, BlockProof, OuterZkvmHost, SerializedAggregatedProof,
};
use sov_rollup_interface::zk::SerializedZkProof;

/// A DA block header paired with the inner-proof data covering that block.
type HeaderWithBlockProof<Address, Da, Root> =
    (<Da as DaSpec>::BlockHeader, BlockProof<Address, Da, Root>);

/// A mock implementing the zkVM trait.
#[derive(Clone)]
pub struct MockZkvmHost {
    notification_manager: NotificationManager,
    wait_for_proof: bool,
    /// Anchor extracted from the most recently produced aggregated proof.
    /// Shared across clones so that continuity assertions in
    /// [`OuterZkvmHost::run_proof_aggregation`] hold across the whole prover
    /// service.
    previous_anchor: Arc<Mutex<Option<PreviousAggregatedProofAnchor>>>,
}

/// Continuity anchor extracted from a previously produced aggregated proof.
#[derive(Clone, Debug)]
struct PreviousAggregatedProofAnchor {
    final_slot_number: SlotNumber,
    genesis_state_root: Vec<u8>,
    final_state_root: Vec<u8>,
}

impl PreviousAggregatedProofAnchor {
    fn from_public_data<Address, Da, Root>(
        public_data: &AggregatedProofPublicData<Address, Da, Root>,
    ) -> Self
    where
        Address: Serialize,
        Da: DaSpec,
        Root: Serialize,
    {
        Self {
            final_slot_number: public_data.final_slot_number,
            genesis_state_root: bincode::serialize(&public_data.genesis_state_root)
                .expect("genesis_state_root must be bincode-serializable"),
            final_state_root: bincode::serialize(&public_data.final_state_root)
                .expect("final_state_root must be bincode-serializable"),
        }
    }

    fn deserialize_genesis_state_root<Root: DeserializeOwned>(&self) -> Root {
        bincode::deserialize(&self.genesis_state_root)
            .expect("genesis_state_root must be bincode-deserializable")
    }

    fn deserialize_final_state_root<Root: DeserializeOwned>(&self) -> Root {
        bincode::deserialize(&self.final_state_root)
            .expect("final_state_root must be bincode-deserializable")
    }
}

impl MockZkvmHost {
    /// Creates a new MockZkvm.
    pub fn new() -> Self {
        Self {
            wait_for_proof: true,
            notification_manager: Default::default(),
            previous_anchor: Arc::new(Mutex::new(None)),
        }
    }

    /// Creates a new MockZkvm, the `ZkvmHost::add_hint_and_run` will return immediately.
    pub fn new_non_blocking() -> Self {
        Self {
            wait_for_proof: false,
            notification_manager: Default::default(),
            previous_anchor: Arc::new(Mutex::new(None)),
        }
    }

    /// Like [`Self::new_non_blocking`], but seeded with the public data of the
    /// latest verified aggregated proof previously persisted in the ledger DB
    /// so that continuity assertions in
    /// [`OuterZkvmHost::run_proof_aggregation`] survive a node restart.
    pub fn new_non_blocking_with_previous_anchor<Address, Da, Root>(
        previous_public_data: Option<&AggregatedProofPublicData<Address, Da, Root>>,
    ) -> Self
    where
        Address: Serialize,
        Da: DaSpec,
        Root: Serialize,
    {
        let previous_anchor =
            previous_public_data.map(PreviousAggregatedProofAnchor::from_public_data);

        Self {
            wait_for_proof: false,
            notification_manager: Default::default(),
            previous_anchor: Arc::new(Mutex::new(previous_anchor)),
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

    /// Mirrors the per-inner-proof continuity checks performed by the real
    /// aggregation circuit. Verifies that consecutive `BlockProof.st` entries
    /// have:
    ///   1. slot numbers that increment by exactly one,
    ///   2. a `slot_hash` matching the paired DA block header's hash,
    ///   3. a `prev_hash` pointing at the predecessor DA block's hash,
    ///   4. an `initial_state_root` equal to the predecessor's `final_state_root`.
    fn check_inner_proof_chain<Address, Da: DaSpec, Root: PartialEq + Debug>(
        headers_with_block_proofs: &[HeaderWithBlockProof<Address, Da, Root>],
    ) {
        let mut prev: Option<(SlotNumber, &Da::SlotHash, &Root)> = None;
        for (index, (header, bp)) in headers_with_block_proofs.iter().enumerate() {
            let header_hash = header.hash();
            assert_eq!(
                header_hash, bp.st.slot_hash,
                "Slot hash mismatch at index {index}: DA block header hash doesn't match inner-proof public data",
            );
            if let Some((prev_slot, prev_hash, prev_state_root)) = prev {
                let expected = prev_slot.next();
                assert_eq!(
                    bp.st.slot_number, expected,
                    "Slot number discontinuity at index {index}: expected {expected}, got {}",
                    bp.st.slot_number,
                );
                assert_eq!(
                    prev_hash,
                    &header.prev_hash(),
                    "DA block chain broken at index {index}: prev_hash mismatch",
                );
                assert_eq!(
                    &bp.st.initial_state_root, prev_state_root,
                    "State root discontinuity at index {index}: previous final_state_root != current initial_state_root",
                );
            }
            prev = Some((bp.st.slot_number, &bp.st.slot_hash, &bp.st.final_state_root));
        }
    }

    /// Sleeps for the duration (in milliseconds) read from `env_var`. Used in
    /// tests/soak runs to throttle the otherwise-instant mock prover. No-op
    /// if the env var is unset or not parseable.
    fn maybe_mock_sleep(env_var: &str) {
        let Some(ms) = std::env::var(env_var)
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
        else {
            return;
        };
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }

    /// Polls `env_var` every 10ms and blocks while it is set to
    /// `"STOP_PROVING"`. Returns immediately once the var is unset.
    fn wait_while_stop_proving(env_var: &str) {
        while std::env::var(env_var).as_deref() == Ok("STOP_PROVING") {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
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
        Self::maybe_mock_sleep("SOV_MOCK_PROVE_SLEEP_MS");
        self.add_hint_and_run_inner(item)
            .map(|raw_proof| SerializedZkProof { raw_proof })
    }
}

impl OuterZkvmHost for MockZkvmHost {
    fn run_proof_aggregation<
        Address: Serialize + Clone,
        Da: DaSpec,
        Root: Serialize + DeserializeOwned + Clone + PartialEq + Debug,
    >(
        &self,
        _genesis_state_root: Root,
        headers_with_block_proofs: Vec<(Da::BlockHeader, BlockProof<Address, Da, Root>)>,
    ) -> anyhow::Result<SerializedAggregatedProof> {
        Self::maybe_mock_sleep("SOV_MOCK_AGGREGATION_SLEEP_MS");
        Self::wait_while_stop_proving("SOV_MOCK_AGGREGATION_GATE");

        let mut previous = self
            .previous_anchor
            .lock()
            .expect("previous_anchor mutex was poisoned");

        // Mirror the checks performed by the real aggregation circuit
        // (`run_aggregation_program` in sov-rollup-interface).
        Self::check_inner_proof_chain(&headers_with_block_proofs);

        let block_proofs_data = headers_with_block_proofs
            .iter()
            .map(|(_, bp)| bp)
            .collect::<Vec<_>>();

        let genesis_state_root = if let Some(prev) = previous.as_ref() {
            prev.deserialize_genesis_state_root()
        } else {
            block_proofs_data
                .get(0)
                .unwrap()
                .st
                .initial_state_root
                .clone()
        };

        let public_data = AggregatedProofPublicData::from_block_proofs(
            block_proofs_data.as_slice(),
            genesis_state_root,
        );

        if let Some(prev) = previous.as_ref() {
            assert_eq!(
                public_data.initial_slot_number,
                prev.final_slot_number.next(),
                "Aggregated proof continuity violated: new aggregation starts at slot {} but previous aggregation ended at slot {}",
                public_data.initial_slot_number,
                prev.final_slot_number,
            );

            let prev_genesis_state_root: Root = prev.deserialize_genesis_state_root();
            let prev_final_state_root: Root = prev.deserialize_final_state_root();

            assert_eq!(
                public_data.genesis_state_root, prev_genesis_state_root,
                "Aggregated proof continuity violated: genesis_state_root differs from previous aggregation",
            );
            assert_eq!(
                public_data.initial_state_root, prev_final_state_root,
                "Aggregated proof continuity violated: new initial_state_root does not match previous final_state_root",
            );
        } else {
            //assert_eq!(public_data.initial_slot_number, SlotNumber::ONE);
        }

        let serialized = self
            .add_hint_and_run_inner(&public_data)
            .map(|raw_aggregated_proof| SerializedAggregatedProof {
                raw_aggregated_proof,
            })?;

        *previous = Some(PreviousAggregatedProofAnchor::from_public_data(
            &public_data,
        ));
        Ok(serialized)
    }
}
