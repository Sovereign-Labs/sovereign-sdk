//! Defines types that are related to the `AggregatedProof`.
/// Core aggregation circuit logic.
pub mod circuit;
/// Common types shared between the aggregated proof program and the host script.
pub mod common;

use core::marker::PhantomData;

use borsh::{BorshDeserialize, BorshSerialize};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use super::{StateTransitionPublicData, ZkVerifier};
use crate::common::{SafeVec, SlotNumber};
use crate::da::DaSpec;
use crate::zk::SerializedZkProof;

/// Host-side interface for the outer zkVM that produces aggregated proofs.
pub trait OuterZkvmHost: Clone + Send + Sync + 'static {
    /// Aggregates per-block inner proofs into a single serialized aggregated proof.
    fn run_proof_aggregation<
        Address: Serialize + DeserializeOwned + Clone,
        Da: DaSpec,
        Root: Serialize + DeserializeOwned + Clone + PartialEq + core::fmt::Debug,
    >(
        &self,
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
pub struct CodeCommitmentHash(pub(crate) SafeVec<u8, 32>);

impl Default for CodeCommitmentHash {
    fn default() -> Self {
        // We use [0u8; 32] to match the placeholder value in the chain_state genesis.
        // This remains the default until the full proof aggregation workflow is finalized.
        Self::from_u8_array([0u8; Self::HASH_LEN])
    }
}

impl From<[u8; CodeCommitmentHash::HASH_LEN]> for CodeCommitmentHash {
    fn from(bytes: [u8; CodeCommitmentHash::HASH_LEN]) -> Self {
        Self::from_u8_array(bytes)
    }
}

/// Error returned when decoding a [`CodeCommitmentHash`] into a concrete
/// [`crate::zk::CodeCommitmentTrait`] implementation fails.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CodeCommitmentDecodeError {
    /// The hash length did not match the adapter's expected length.
    #[error("invalid code commitment hash length: expected {expected}, got {got}")]
    InvalidLength {
        /// Expected byte length (typically [`CodeCommitmentHash::HASH_LEN`]).
        expected: usize,
        /// Actual byte length supplied.
        got: usize,
    },
}

impl CodeCommitmentHash {
    /// Canonical byte length expected by every
    /// [`crate::zk::CodeCommitmentTrait`] implementation. Must match the
    /// `SafeVec` capacity in the struct definition above.
    pub const HASH_LEN: usize = 32;

    /// Byte length of the inner buffer. May be less than [`Self::HASH_LEN`]
    /// for hashes decoded from untrusted bytes — the [`SafeVec`] only caps
    /// the upper bound.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns `true` if the inner buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Creates a [`CodeCommitmentHash`] from a `[u8; HASH_LEN]` array. This is
    /// the infallible primary constructor — the fixed-size input statically
    /// guarantees the inner `SafeVec` invariant.
    pub fn from_u8_array(bytes: [u8; Self::HASH_LEN]) -> Self {
        Self(
            bytes
                .to_vec()
                .try_into()
                .expect("HASH_LEN bytes always fit in SafeVec<u8, HASH_LEN>"),
        )
    }

    /// Creates a [`CodeCommitmentHash`] from a `[u32; 8]` array using big-endian byte order.
    /// This matches the representation used by SP1's `HashableKey::hash_bytes`.
    pub fn from_u32_array(arr: [u32; 8]) -> Self {
        let mut bytes = [0u8; Self::HASH_LEN];
        for (idx, word) in arr.iter().enumerate() {
            bytes[idx * 4..(idx + 1) * 4].copy_from_slice(&word.to_be_bytes());
        }
        Self::from_u8_array(bytes)
    }

    /// Converts this hash back to a `[u32; 8]` array using big-endian byte
    /// order, returning [`CodeCommitmentDecodeError::InvalidLength`] if the
    /// byte count is not canonical.
    pub fn to_u32_array(&self) -> Result<[u32; 8], CodeCommitmentDecodeError> {
        if self.0.len() != Self::HASH_LEN {
            return Err(CodeCommitmentDecodeError::InvalidLength {
                expected: Self::HASH_LEN,
                got: self.0.len(),
            });
        }
        let mut arr = [0u32; 8];
        for (idx, chunk) in self.0.chunks_exact(4).enumerate() {
            arr[idx] = u32::from_be_bytes(chunk.try_into().unwrap());
        }
        Ok(arr)
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
#[derive(Debug, Eq, PartialEq, Serialize, Deserialize, Clone, BorshDeserialize, BorshSerialize)]
pub struct AggregatedProofPublicData<Address, Da: DaSpec, Root> {
    /// Initial rollup slot.
    pub initial_slot_number: SlotNumber,
    /// Final rollup slot.
    pub final_slot_number: SlotNumber,
    /// The slot number that [`Self::origin_state_root`] corresponds to: the slot
    /// at the end of which the chain's origin state root was produced. When the
    /// chain extends unbroken from the rollup's genesis, this equals
    /// [`SlotNumber::GENESIS`].
    pub origin_slot_number: SlotNumber,
    /// The origin state root of the aggregated proof: the initial state root of the
    /// first inner proof in this aggregation's chain. When the chain extends unbroken
    /// from the rollup's genesis, this equals the rollup's genesis state root.
    pub origin_state_root: Root,
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

impl<Address, Da: DaSpec, Root: AsRef<[u8]>> core::fmt::Display
    for AggregatedProofPublicData<Address, Da, Root>
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "AggregatedProofPublicData(initial_slot_number: {}, final_slot_number: {}, origin_slot_number: {}, origin_state_root: {}, initial_state_root: 0x{}, final_state_root: 0x{}, initial_slot_hash: 0x{}, final_slot_hash: 0x{}, inner_vkey_hash: {}, outer_vk_hash: {})",
            self.initial_slot_number,
            self.final_slot_number,
            self.origin_slot_number,
            hex::encode(self.origin_state_root.as_ref()),
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
