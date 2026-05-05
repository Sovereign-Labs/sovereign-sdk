//! Defines types that are related to the `AggregatedProof`.
/// Core aggregation circuit logic.
#[cfg(target_os = "zkvm")]
pub mod circuit;
/// Common types shared between the aggregated proof program and the host script.
pub mod common;

use core::marker::PhantomData;

use borsh::{BorshDeserialize, BorshSerialize};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use super::{StateTransitionPublicData, ZkVerifier};
use crate::common::SlotNumber;
use crate::da::DaSpec;
use crate::zk::SerializedZkProof;

/// Host-side interface for the outer zkVM that produces aggregated proofs.
pub trait OuterZkvmHost: Clone + Send + Sync + 'static {
    /// Aggregates per-block inner proofs into a single serialized aggregated proof.
    fn run_proof_aggregation<
        Address: Serialize + Clone,
        Da: DaSpec,
        Root: Serialize + DeserializeOwned + Clone + PartialEq + core::fmt::Debug,
    >(
        &self,
        genesis_state_root: Root,
        headers_with_block_proofs: Vec<(Da::BlockHeader, BlockProof<Address, Da, Root>)>,
    ) -> anyhow::Result<SerializedAggregatedProof>;
}

/// A single block's proof data, used to build an [`AggregatedProofPublicData`].
#[derive(Clone)]
pub struct BlockProof<Address, Da: DaSpec, Root> {
    /// The raw proof bytes.
    pub proof: SerializedZkProof,
    /// The state transition public data for this block.
    pub st: StateTransitionPublicData<Address, Da, Root>,
}

/// A code commitment hash used to identify ZK circuits (both inner and outer).
#[derive(Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize, Serialize, Deserialize, Clone)]
pub struct CodeCommitmentHash(pub Vec<u8>);

impl Default for CodeCommitmentHash {
    fn default() -> Self {
        // We use [0u8; 32] to match the placeholder value in the chain_state genesis.
        // This remains the default until the full proof aggregation workflow is finalized.
        Self(vec![0u8; 32])
    }
}

impl CodeCommitmentHash {
    /// Creates a [`CodeCommitmentHash`] from a `[u32; 8]` array using big-endian byte order.
    /// This matches the representation used by SP1's `HashableKey::hash_bytes`.
    pub fn from_u32_array(arr: [u32; 8]) -> Self {
        let mut bytes = Vec::with_capacity(32);
        for word in arr {
            bytes.extend_from_slice(&word.to_be_bytes());
        }
        Self(bytes)
    }

    /// Converts this hash back to a `[u32; 8]` array using big-endian byte order.
    ///
    /// # Panics
    ///
    /// Panics if the inner byte vector is not exactly 32 bytes long.
    pub fn to_u32_array(&self) -> [u32; 8] {
        assert_eq!(self.0.len(), 32, "CodeCommitmentHash must be 32 bytes");
        let mut arr = [0u32; 8];
        for (idx, chunk) in self.0.chunks_exact(4).enumerate() {
            arr[idx] = u32::from_be_bytes(chunk.try_into().unwrap());
        }
        arr
    }
}

impl core::fmt::Display for CodeCommitmentHash {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.0.is_empty() {
            return write!(f, "CodeCommitmentHash([])");
        }
        write!(f, "CodeCommitmentHash(0x{})", hex::encode(&self.0))
    }
}

/// Public data of an aggregated proof.
#[derive(Debug, Eq, PartialEq, Serialize, Deserialize, Clone)]
pub struct AggregatedProofPublicData<Address, Da: DaSpec, Root> {
    /// Initial rollup slot.
    pub initial_slot_number: SlotNumber,
    /// Final rollup slot.
    pub final_slot_number: SlotNumber,
    /// The genesis state root of the aggregated proof.
    pub genesis_state_root: Root,
    /// The initial state root of the aggregated proof.
    pub initial_state_root: Root,
    /// The final state root of the aggregated proof.
    pub final_state_root: Root,
    /// The initial slot hash of the aggregated proof.
    pub initial_slot_hash: Da::SlotHash,
    /// The final slot hash of the aggregated proof.
    pub final_slot_hash: Da::SlotHash,
    /// Inner verifying key hash of the aggregated proof circuit.
    pub inner_vkey_hash: CodeCommitmentHash,
    /// Outer verifying key hash of the aggregated proof circuit.
    pub outer_vk_hash: CodeCommitmentHash,
    /// These are the addresses of the provers who proved individual blocks.
    pub rewarded_addresses: Vec<Address>,
}

impl<Address: Clone, Da: DaSpec, Root: Clone> AggregatedProofPublicData<Address, Da, Root>
where
    Da::SlotHash: Clone,
{
    /// Constructs an [`AggregatedProofPublicData`] from a slice of [`BlockProof`] references,
    /// deriving initial/final fields from the first and last entries.
    pub fn from_block_proofs(
        block_proofs: &[&BlockProof<Address, Da, Root>],
        genesis_state_root: Root,
    ) -> Self {
        let initial = block_proofs
            .first()
            .expect("block_proofs must not be empty");
        let final_bp = block_proofs.last().expect("block_proofs must not be empty");
        let rewarded_addresses = block_proofs
            .iter()
            .map(|bp| bp.st.prover_address.clone())
            .collect();
        Self {
            rewarded_addresses,
            initial_slot_number: initial.st.slot_number,
            final_slot_number: final_bp.st.slot_number,
            genesis_state_root,
            initial_state_root: initial.st.initial_state_root.clone(),
            final_state_root: final_bp.st.final_state_root.clone(),
            initial_slot_hash: initial.st.slot_hash.clone(),
            final_slot_hash: final_bp.st.slot_hash.clone(),
            // This is used only for mock proving and matches the values in the chain_state genesis.
            inner_vkey_hash: CodeCommitmentHash::default(),
            outer_vk_hash: CodeCommitmentHash::default(),
        }
    }
}

impl<Address, Da: DaSpec, Root: AsRef<[u8]>> core::fmt::Display
    for AggregatedProofPublicData<Address, Da, Root>
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "AggregatedProofPublicData(initial_slot_number: {}, final_slot_number: {}, genesis_state_root: {}, initial_state_root: 0x{}, final_state_root: 0x{}, initial_slot_hash: 0x{}, final_slot_hash: 0x{}, inner_vkey_hash: {}, outer_vk_hash: {})",
            self.initial_slot_number,
            self.final_slot_number,
            hex::encode(self.genesis_state_root.as_ref()),
            hex::encode(self.initial_state_root.as_ref()),
            hex::encode(self.final_state_root.as_ref()),
            hex::encode(self.initial_slot_hash.as_ref()),
            hex::encode(self.final_slot_hash.as_ref()),
            self.inner_vkey_hash,
            self.outer_vk_hash
        )
    }
}

/// Represents a serialized aggregated proof.
#[derive(
    Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize, Serialize, Deserialize, Clone, Default,
)]
pub struct SerializedAggregatedProof {
    /// Serialized proof.
    pub raw_aggregated_proof: Vec<u8>,
}

impl SerializedAggregatedProof {
    /// Converts a [`SerializedAggregatedProof`] into a [`SerializedZkProof`].
    pub fn to_serialized_zk_proof(self) -> SerializedZkProof {
        SerializedZkProof {
            raw_proof: self.raw_aggregated_proof,
        }
    }
}

/// A serialized partial proof receipt.
#[derive(
    Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize, Serialize, Deserialize, Clone, Default,
)]
pub struct SerializedPartialProofReceipt {
    /// Serialized proof receipt.
    pub raw_proof_receipt: Vec<u8>,
}

/// Validates an Aggregated Proof.
pub struct AggregateProofVerifier<Vm: ZkVerifier> {
    _vm: PhantomData<Vm>,
    outer_proof_code_commitment: Vm::CodeCommitment,
}

impl<Vm: ZkVerifier> AggregateProofVerifier<Vm> {
    /// Creates a new `AggregateProofVerifier`.
    pub fn new(outer_proof_code_commitment: Vm::CodeCommitment) -> Self {
        Self {
            _vm: PhantomData,
            outer_proof_code_commitment,
        }
    }

    /// Verifies whether an [`SerializedAggregatedProof`] contains a valid proof.
    pub fn verify<Address: DeserializeOwned, Da: DaSpec, Root: DeserializeOwned>(
        &self,
        proof_data: &SerializedAggregatedProof,
    ) -> Result<AggregatedProofPublicData<Address, Da, Root>, Vm::Error> {
        let public_data = Vm::verify_with_proof::<AggregatedProofPublicData<Address, Da, Root>>(
            &proof_data.clone().to_serialized_zk_proof(),
            &self.outer_proof_code_commitment,
        )?;

        Ok(public_data)
    }
}

/// A DA block header bundled with its corresponding serialized proof.
#[derive(Debug, serde::Serialize, serde::Deserialize, Clone)]
pub struct BlockHeaderWithProof<Da: crate::da::DaSpec> {
    /// The DA layer block header associated with this proof.
    pub da_block_header: Da::BlockHeader,
    /// The serialized proof bytes.
    pub proof: SerializedZkProof,
}
