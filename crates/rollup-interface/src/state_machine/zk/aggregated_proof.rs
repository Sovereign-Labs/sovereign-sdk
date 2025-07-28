//! Defines types that are related to the `AggregatedProof`.
use core::marker::PhantomData;

use borsh::{BorshDeserialize, BorshSerialize};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use super::ZkVerifier;
use crate::common::SlotNumber;
use crate::da::DaSpec;

/// Aggregated proof code commitment.
#[derive(
    Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize, Serialize, Deserialize, Clone, Default,
)]
pub struct CodeCommitment(pub Vec<u8>);

impl core::fmt::Display for CodeCommitment {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.0.is_empty() {
            return write!(f, "CodeCommitment([])");
        }
        write!(f, "CodeCommitment(0x{})", hex::encode(&self.0))
    }
}

/// Public data of an aggregated proof.
#[derive(Debug, Eq, PartialEq, Serialize, Deserialize, Clone)]
pub struct AggregatedProofPublicData<Address, Da: DaSpec, Root> {
    /// Initial rollup height.
    pub initial_slot_number: SlotNumber,
    /// Final rollup height.
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
    /// Code Commitment of the aggregated proof circuit.
    pub code_commitment: CodeCommitment,
    /// These are the addresses of the provers who proved individual blocks.
    pub rewarded_addresses: Vec<Address>,
}

impl<Address, Da: DaSpec, Root: AsRef<[u8]>> core::fmt::Display
    for AggregatedProofPublicData<Address, Da, Root>
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "AggregatedProofPublicData(initial_slot_number: {}, final_slot_number: {}, genesis_state_root: {}, initial_state_root: 0x{}, final_state_root: 0x{}, initial_slot_hash: 0x{}, final_slot_hash: 0x{}, code_commitment: {})",
            self.initial_slot_number,
            self.final_slot_number,
            hex::encode(self.genesis_state_root.as_ref()),
            hex::encode(self.initial_state_root.as_ref()),
            hex::encode(self.final_state_root.as_ref()),
            hex::encode(self.initial_slot_hash.as_ref()),
            hex::encode(self.final_slot_hash.as_ref()),
            self.code_commitment
        )
    }
}

/// Represents a serialized aggregated proof.
#[derive(Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize, Serialize, Deserialize, Clone)]
pub struct SerializedAggregatedProof {
    /// Serialized proof.
    pub raw_aggregated_proof: Vec<u8>,
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
        let public_data = Vm::verify::<AggregatedProofPublicData<Address, Da, Root>>(
            proof_data.raw_aggregated_proof.as_slice(),
            &self.outer_proof_code_commitment,
        )?;

        Ok(public_data)
    }
}
