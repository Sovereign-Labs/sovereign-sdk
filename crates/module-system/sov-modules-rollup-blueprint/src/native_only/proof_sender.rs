//! Standard implementation of [`ProofSender`].

use std::sync::Arc;

use async_trait::async_trait;
use borsh::{BorshDeserialize, BorshSerialize};
use sov_modules_api::proof_metadata::{ProofType, SerializeProofWithDetails};
use sov_modules_api::transaction::{PriorityFeeBips, TxDetails};
use sov_modules_api::{Amount, ProofSender, Spec};
use sov_rollup_interface::common::SlotNumber;
use sov_rollup_interface::optimistic::{SerializedAttestation, SerializedChallenge};
use sov_rollup_interface::stf::BlobSenderStatus;
use sov_rollup_interface::zk::aggregated_proof::SerializedAggregatedProof;
use sov_sequencer::{ProofBlobSender, SerializedProofWithDetailsBytes};

const MAX_FEE: Amount = Amount::new(100_000_000);

/// Adds metadata about gas & fees to the proof blob.
pub struct SovApiProofSender<S: Spec> {
    _phantom: std::marker::PhantomData<S>,
    inner: Arc<dyn ProofBlobSender>,
    /// The rollup's chain id, stamped into the proof-blob tx details.
    /// Supplied by the caller from `Runtime::chain_id()` (this type implements
    /// the runtime-agnostic `ProofSender` trait, so it cannot read it itself).
    chain_id: u64,
}

impl<S: Spec> SovApiProofSender<S> {
    /// Creates a new [`SovApiProofSender`]. `chain_id` should come from
    /// `Runtime::chain_id()`.
    pub fn new(inner: Arc<dyn ProofBlobSender>, chain_id: u64) -> Self {
        Self {
            _phantom: Default::default(),
            inner,
            chain_id,
        }
    }
}

#[async_trait]
impl<S: Spec> ProofSender for SovApiProofSender<S> {
    async fn publish_proof_blob_with_metadata(
        &self,
        serialized_proof: SerializedAggregatedProof,
    ) -> anyhow::Result<()> {
        let proof_data = serialize_proof_blob_with_metadata::<S>(serialized_proof, self.chain_id)?;
        self.inner.produce_and_publish_proof_blob(proof_data).await
    }

    async fn publish_attestation_blob_with_metadata(
        &self,
        serialized_attestation: SerializedAttestation,
    ) -> anyhow::Result<()> {
        let proof_data =
            serialize_attestation_blob_with_metadata::<S>(serialized_attestation, self.chain_id)?;
        self.inner.produce_and_publish_proof_blob(proof_data).await
    }

    async fn publish_challenge_blob_with_metadata(
        &self,
        serialized_challenge: SerializedChallenge,
        slot_height: SlotNumber,
    ) -> anyhow::Result<()> {
        let proof_data = serialize_challenge_blob_with_metadata::<S>(
            serialized_challenge,
            slot_height,
            self.chain_id,
        )?;
        self.inner.produce_and_publish_proof_blob(proof_data).await
    }

    async fn proof_blob_sender_status(&self) -> anyhow::Result<BlobSenderStatus> {
        self.inner.proof_blob_sender_status().await
    }
}

/// See [`ProofSender::publish_attestation_blob_with_metadata`].
/// `chain_id` should come from `Runtime::chain_id()`.
pub fn serialize_attestation_blob_with_metadata<S: Spec>(
    serialized_attestation: SerializedAttestation,
    chain_id: u64,
) -> anyhow::Result<SerializedProofWithDetailsBytes> {
    let proof_with_details = SerializeProofWithDetails::<S> {
        proof: ProofType::OptimisticProofAttestation(serialized_attestation),
        details: make_details(MAX_FEE, chain_id),
    };

    Ok(SerializedProofWithDetailsBytes(
        borsh::to_vec(&proof_with_details)?.into(),
    ))
}

/// See [`ProofSender::publish_challenge_blob_with_metadata`].
/// `chain_id` should come from `Runtime::chain_id()`.
pub fn serialize_challenge_blob_with_metadata<S: Spec>(
    serialized_challenge: SerializedChallenge,
    slot_height: SlotNumber,
    chain_id: u64,
) -> anyhow::Result<SerializedProofWithDetailsBytes> {
    let proof_with_details = SerializeProofWithDetails::<S> {
        proof: ProofType::OptimisticProofChallenge(serialized_challenge, slot_height),
        details: make_details(MAX_FEE, chain_id),
    };

    Ok(SerializedProofWithDetailsBytes(
        borsh::to_vec(&proof_with_details)?.into(),
    ))
}

/// See [`ProofSender::publish_proof_blob_with_metadata`].
/// `chain_id` should come from `Runtime::chain_id()`.
pub fn serialize_proof_blob_with_metadata<S: Spec>(
    serialized_proof: SerializedAggregatedProof,
    chain_id: u64,
) -> anyhow::Result<SerializedProofWithDetailsBytes> {
    let proof_with_details = SerializeProofWithDetails::<S> {
        proof: ProofType::ZkAggregatedProof(serialized_proof),
        details: make_details(MAX_FEE, chain_id),
    };

    Ok(SerializedProofWithDetailsBytes(
        borsh::to_vec(&proof_with_details)?.into(),
    ))
}

fn make_details<S: Spec>(max_fee: Amount, chain_id: u64) -> TxDetails<S> {
    TxDetails {
        max_priority_fee_bips: PriorityFeeBips::ZERO,
        max_fee,
        gas_limit: None,
        chain_id,
    }
}

#[derive(Debug, PartialEq, Clone, BorshDeserialize, BorshSerialize)]
#[allow(dead_code)]
struct PreferredProofData {
    sequence_number: u64,
    data: Vec<u8>,
}
