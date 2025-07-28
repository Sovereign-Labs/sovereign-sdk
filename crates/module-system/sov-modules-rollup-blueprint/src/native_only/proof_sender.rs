//! Standard implementation of [`ProofSender`].

use std::sync::Arc;

use async_trait::async_trait;
use borsh::{BorshDeserialize, BorshSerialize};
use sov_modules_api::capabilities::config_chain_id;
use sov_modules_api::proof_metadata::{ProofType, SerializeProofWithDetails};
use sov_modules_api::transaction::{PriorityFeeBips, TxDetails};
use sov_modules_api::{Amount, ProofSender, Spec};
use sov_rollup_interface::common::SlotNumber;
use sov_rollup_interface::optimistic::{SerializedAttestation, SerializedChallenge};
use sov_rollup_interface::zk::aggregated_proof::SerializedAggregatedProof;
use sov_sequencer::ProofBlobSender;

const MAX_FEE: Amount = Amount::new(10_000_000);

/// Adds metadata about gas & fees to the proof blob.
pub struct SovApiProofSender<S: Spec> {
    _phantom: std::marker::PhantomData<S>,
    inner: Arc<dyn ProofBlobSender>,
}

impl<S: Spec> SovApiProofSender<S> {
    /// Creates a new [`SovApiProofSender`].
    pub fn new(inner: Arc<dyn ProofBlobSender>) -> Self {
        Self {
            _phantom: Default::default(),
            inner,
        }
    }
}

#[async_trait]
impl<S: Spec> ProofSender for SovApiProofSender<S> {
    async fn publish_proof_blob_with_metadata(
        &self,
        serialized_proof: SerializedAggregatedProof,
    ) -> anyhow::Result<()> {
        let proof_data = serialize_proof_blob_with_metadata::<S>(serialized_proof)?;
        self.inner
            .produce_and_publish_proof_blob(proof_data)
            .await?;

        Ok(())
    }

    async fn publish_attestation_blob_with_metadata(
        &self,
        serialized_attestation: SerializedAttestation,
    ) -> anyhow::Result<()> {
        let proof_data = serialize_attestation_blob_with_metadata::<S>(serialized_attestation)?;
        self.inner
            .produce_and_publish_proof_blob(proof_data)
            .await?;

        Ok(())
    }

    async fn publish_challenge_blob_with_metadata(
        &self,
        serialized_challenge: SerializedChallenge,
        slot_height: SlotNumber,
    ) -> anyhow::Result<()> {
        let proof_data =
            serialize_challenge_blob_with_metadata::<S>(serialized_challenge, slot_height)?;
        self.inner
            .produce_and_publish_proof_blob(proof_data)
            .await?;

        Ok(())
    }
}

/// See [`ProofSender::publish_attestation_blob_with_metadata`].
pub fn serialize_attestation_blob_with_metadata<S: Spec>(
    serialized_attestation: SerializedAttestation,
) -> anyhow::Result<Arc<[u8]>> {
    let proof_with_details = SerializeProofWithDetails::<S> {
        proof: ProofType::OptimisticProofAttestation(serialized_attestation),
        details: make_details(MAX_FEE),
    };

    Ok(borsh::to_vec(&proof_with_details)?.into())
}

/// See [`ProofSender::publish_challenge_blob_with_metadata`].
pub fn serialize_challenge_blob_with_metadata<S: Spec>(
    serialized_challenge: SerializedChallenge,
    slot_height: SlotNumber,
) -> anyhow::Result<Arc<[u8]>> {
    let proof_with_details = SerializeProofWithDetails::<S> {
        proof: ProofType::OptimisticProofChallenge(serialized_challenge, slot_height),
        details: make_details(MAX_FEE),
    };

    Ok(borsh::to_vec(&proof_with_details)?.into())
}

/// See [`ProofSender::publish_proof_blob_with_metadata`].
pub fn serialize_proof_blob_with_metadata<S: Spec>(
    serialized_proof: SerializedAggregatedProof,
) -> anyhow::Result<Arc<[u8]>> {
    let proof_with_details = SerializeProofWithDetails::<S> {
        proof: ProofType::ZkAggregatedProof(serialized_proof),
        details: make_details(MAX_FEE),
    };

    Ok(borsh::to_vec(&proof_with_details)?.into())
}

fn make_details<S: Spec>(max_fee: Amount) -> TxDetails<S> {
    TxDetails {
        max_priority_fee_bips: PriorityFeeBips::ZERO,
        max_fee,
        gas_limit: None,
        chain_id: config_chain_id(),
    }
}

#[derive(Debug, PartialEq, Clone, BorshDeserialize, BorshSerialize)]
struct PreferredProofData {
    sequence_number: u64,
    data: Vec<u8>,
}
