use std::fmt::Debug;
use std::sync::{Arc, Mutex};

use crate::notifier::NotificationManager;
use crate::{MockCodeCommitment, MockProof, MockZkGuest};
use serde::de::DeserializeOwned;
use serde::Serialize;
use sov_rollup_interface::da::DaSpec;
use sov_rollup_interface::zk::aggregated_proof::circuit::run_aggregation_program;
use sov_rollup_interface::zk::aggregated_proof::common::{
    AggregatedProofWitness, DeferredProofInput, PreviousOuterProofWitness, SerializedPubValues,
};
use sov_rollup_interface::zk::aggregated_proof::{
    BlockProof, OuterZkvmHost, SerializedAggregatedProof,
};
use sov_rollup_interface::zk::{CodeCommitmentTrait, SerializedZkProof};

/// A DA block header paired with the inner-proof data covering that block.
type HeaderWithBlockProof<Address, Da, Root> =
    (<Da as DaSpec>::BlockHeader, BlockProof<Address, Da, Root>);

/// A mock implementing the zkVM trait.
#[derive(Clone)]
pub struct MockZkvmHost {
    notification_manager: NotificationManager,
    wait_for_proof: bool,
    /// Most-recently-produced outer aggregated proof.
    previous_outer_proof: Arc<Mutex<Option<SerializedAggregatedProof>>>,
    /// Commitment this host stamps into proofs.
    code_commitment: MockCodeCommitment,
}

impl MockZkvmHost {
    /// Creates a new MockZkvm.
    pub fn new() -> Self {
        Self {
            wait_for_proof: true,
            notification_manager: Default::default(),
            previous_outer_proof: Arc::new(Mutex::new(None)),
            code_commitment: MockCodeCommitment::default(),
        }
    }

    /// Creates a new MockZkvm, the `ZkvmHost::add_hint_and_run` will return immediately.
    pub fn new_non_blocking() -> Self {
        Self {
            wait_for_proof: false,
            notification_manager: Default::default(),
            previous_outer_proof: Arc::new(Mutex::new(None)),
            code_commitment: MockCodeCommitment::default(),
        }
    }

    /// Like [`Self::new_non_blocking`], but seeded with the
    /// most-recently-produced [`SerializedAggregatedProof`] so the recursive
    /// aggregation circuit can verify continuity across a node restart.
    pub fn new_non_blocking_with_previous_outer_proof(
        previous: Option<SerializedAggregatedProof>,
    ) -> Self {
        Self {
            wait_for_proof: false,
            notification_manager: Default::default(),
            previous_outer_proof: Arc::new(Mutex::new(previous)),
            code_commitment: MockCodeCommitment::default(),
        }
    }

    /// Override the [`MockCodeCommitment`] this host stamps into produced proofs.
    pub fn with_code_commitment(mut self, code_commitment: MockCodeCommitment) -> Self {
        self.code_commitment = code_commitment;
        self
    }

    /// Simulates zk proof generation.
    pub fn make_proof(&self) {
        // We notify the worker thread.
        self.notification_manager.notify();
    }

    /// Create a proof for MockZkvm.
    pub fn create_serialized_proof<T: Serialize>(
        is_valid: bool,
        transition: T,
    ) -> SerializedZkProof {
        Self::create_serialized_proof_with_commitment(
            is_valid,
            transition,
            MockCodeCommitment::default(),
        )
    }

    /// Create a proof that pins itself to the given [`MockCodeCommitment`].
    pub fn create_serialized_proof_with_commitment<T: Serialize>(
        is_valid: bool,
        transition: T,
        code_commitment: MockCodeCommitment,
    ) -> SerializedZkProof {
        let data = bincode::serialize(&transition).unwrap();
        let raw_proof = MockProof {
            is_valid,
            pub_data: data,
            code_commitment,
        }
        .serialize()
        .unwrap();
        SerializedZkProof { raw_proof }
    }

    /// Bincode-encodes a [`MockProof`] with `is_valid: true` and the given public-data bytes and commitment.
    fn valid_mock_proof_bytes(
        pub_data: Vec<u8>,
        code_commitment: MockCodeCommitment,
    ) -> anyhow::Result<Vec<u8>> {
        Ok(MockProof {
            is_valid: true,
            pub_data,
            code_commitment,
        }
        .serialize()?)
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

    /// Recovers the inner-guest commitment from the inner proofs being aggregated.
    fn inner_vkey_hash_from_proof(
        proof: &SerializedZkProof,
    ) -> sov_rollup_interface::zk::aggregated_proof::CodeCommitmentHash {
        let mock_proof = MockProof::deserialize(&proof.raw_proof)
            .expect("inner proof must be a bincode-encoded MockProof");
        mock_proof.code_commitment.to_hash()
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
        Ok(self.code_commitment.clone())
    }

    fn add_hint_deferred_and_run<T: Serialize>(
        &mut self,
        _item: &T,
        _agg_proofs: Vec<SerializedAggregatedProof>,
    ) -> anyhow::Result<SerializedZkProof> {
        // With no guest execution, the mock has nothing to commit, so the
        // inner proof carries empty public data. The aggregation step gets
        // the canonical public output side-band via `BlockProof::st`.
        Self::maybe_mock_sleep("SOV_MOCK_PROVE_SLEEP_MS");
        if self.wait_for_proof {
            self.notification_manager.wait();
        }
        let raw_proof = Self::valid_mock_proof_bytes(Vec::new(), self.code_commitment.clone())?;
        Ok(SerializedZkProof { raw_proof })
    }
}

impl OuterZkvmHost for MockZkvmHost {
    fn run_proof_aggregation<
        Address: Serialize + DeserializeOwned + Clone,
        Da: DaSpec,
        Root: Serialize + DeserializeOwned + Clone + PartialEq + Debug,
    >(
        &self,
        headers_with_block_proofs: Vec<HeaderWithBlockProof<Address, Da, Root>>,
    ) -> anyhow::Result<SerializedAggregatedProof> {
        Self::maybe_mock_sleep("SOV_MOCK_AGGREGATION_SLEEP_MS");
        Self::wait_while_stop_proving("SOV_MOCK_AGGREGATION_GATE");

        let mut previous_outer_proof = self
            .previous_outer_proof
            .lock()
            .expect("previous_outer_proof mutex was poisoned");

        let first = headers_with_block_proofs
            .first()
            .expect("at least one inner proof is required for aggregation");
        let inner_vkey_hash = Self::inner_vkey_hash_from_proof(&first.1.proof);
        let outer_vkey_hash = self.code_commitment.to_hash();

        let mut proof_inputs = Vec::with_capacity(headers_with_block_proofs.len());
        for (header, bp) in headers_with_block_proofs {
            let recovered = MockProof::deserialize(&bp.proof.raw_proof)
                .expect("inner proof must be a bincode-encoded MockProof");
            let pub_values =
                Self::valid_mock_proof_bytes(bp.st.serialize()?, recovered.code_commitment)?;
            proof_inputs.push(DeferredProofInput::<Da> {
                public_values: SerializedPubValues { pub_values },
                da_block_header: header,
            });
        }

        let prev_outer_proof_witness =
            previous_outer_proof
                .as_ref()
                .map(|prev| PreviousOuterProofWitness {
                    public_values: SerializedPubValues {
                        pub_values: prev.raw_aggregated_proof.clone(),
                    },
                });

        let witness = AggregatedProofWitness::<Da> {
            proof_inputs,
            inner_vkey_hash,
            outer_vkey_hash,
            prev_outer_proof_witness,
        };

        let serialized_witness = bincode::serialize(&witness)?;
        let guest = MockZkGuest::with_hint(serialized_witness);

        // Runs the shared aggregation circuit natively: verifies inner proofs
        // and the previous outer proof, asserts within-aggregation DA/state
        // continuity, and commits the new `AggregatedProofPublicData` into
        // the guest's commit slot.
        run_aggregation_program::<Address, Da, Root, crate::MockZkVerifier, MockZkGuest>(&guest);

        let committed_public_data = guest
            .take_committed_bytes()
            .expect("aggregation circuit must commit its public data");

        if self.wait_for_proof {
            self.notification_manager.wait();
        }

        let serialized = SerializedAggregatedProof {
            raw_aggregated_proof: Self::valid_mock_proof_bytes(
                committed_public_data,
                self.code_commitment.clone(),
            )?,
        };

        *previous_outer_proof = Some(serialized.clone());

        Ok(serialized)
    }
}
