//! Utilities for building an optimistic state machine
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use sov_universal_wallet::UniversalWallet;

use crate as sov_rollup_interface; // Needed for UniversalWallet, as it requires global paths
use crate::common::SlotNumber;
use crate::da::DaSpec;
use crate::zk::StateTransitionPublicData;

/// A proof that the attester was bonded at the `rollup_height`.
/// For rollups using the `jmt`, this will be a `jmt::SparseMerkleProof`
#[derive(
    Debug,
    Clone,
    BorshSerialize,
    BorshDeserialize,
    Serialize,
    Deserialize,
    Default,
    PartialEq,
    Eq,
    UniversalWallet,
)]
pub struct ProofOfBond<StateProof> {
    /// The slot_number for which the proof of bond applies
    pub claimed_slot_number: SlotNumber,
    /// The actual state proof that the attester was bonded
    #[sov_wallet(hidden)]
    pub proof: StateProof,
}

/// Service that knows how to generate a [`ProofOfBond`] for a given slot number.
pub trait BondingProofService: Send + Sync + 'static {
    /// The actual state proof that the attester was bonded.
    type StateProof: BorshSerialize + BorshDeserialize + Send + Sync;
    /// Gets the bonding proof for the given slot.
    fn get_bonding_proof(&self, slot_number: SlotNumber) -> Option<ProofOfBond<Self::StateProof>>;
}

/// An attestation that a particular DA layer block transitioned the rollup state to some value
#[derive(
    Debug,
    Clone,
    BorshSerialize,
    BorshDeserialize,
    Serialize,
    Deserialize,
    Default,
    PartialEq,
    Eq,
    UniversalWallet,
)]
pub struct Attestation<SlotHash, StateRoot, StateProof> {
    /// The alleged state root before applying the contents of the da block
    pub initial_state_root: StateRoot,
    /// The hash of the block in which the transition occurred
    pub slot_hash: SlotHash,
    /// The alleged post-state root
    pub post_state_root: StateRoot,
    /// A proof that the attester was bonded at some point in time before the attestation is generated
    pub proof_of_bond: ProofOfBond<StateProof>,
}

/// The contents of a challenge to an attestation, which are contained as a public output of the proof
/// Generic over an address type
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    BorshSerialize,
    BorshDeserialize,
    Serialize,
    Deserialize,
    UniversalWallet,
)]
pub struct ChallengeContents<Address, Da: DaSpec, Root> {
    /// The rollup address of the originator of this challenge
    pub challenger_address: Address,
    /// The state transition that was proven
    #[borsh(bound(
        serialize = "Address: borsh::ser::BorshSerialize, Root: borsh::ser::BorshSerialize, Da::SlotHash: borsh::ser::BorshSerialize",
        deserialize = "Address: borsh::ser::BorshSerialize, Root: borsh::de::BorshDeserialize, Da::SlotHash: borsh::de::BorshDeserialize"
    ))]
    pub state_transition: StateTransitionPublicData<Address, Da, Root>,
}

#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, Serialize, Deserialize, UniversalWallet)]
/// This struct contains the challenge as a raw blob
pub struct Challenge<'a>(&'a [u8]);

/// Represents a serialized attestation.
#[derive(Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize, Serialize, Deserialize, Clone)]
pub struct SerializedAttestation {
    /// Serialized attestation.
    pub raw_attestation: Vec<u8>,
}

/// Represents a serialized challenge.
#[derive(Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize, Serialize, Deserialize, Clone)]
pub struct SerializedChallenge {
    /// Serialized challenge.
    pub raw_challenge: Vec<u8>,
}

impl SerializedAttestation {
    /// Serializes an attestation.
    pub fn from_attestation<
        SlotHash: BorshSerialize,
        StateRoot: BorshSerialize,
        StateProof: BorshSerialize,
    >(
        attestation: &Attestation<SlotHash, StateRoot, StateProof>,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            raw_attestation: borsh::to_vec(attestation)?,
        })
    }

    /// Deserializes an attestation.
    pub fn to_attestation<
        SlotHash: BorshDeserialize,
        StateRoot: BorshDeserialize,
        StateProof: BorshDeserialize,
    >(
        &self,
    ) -> anyhow::Result<Attestation<SlotHash, StateProof, StateRoot>> {
        Ok(borsh::from_slice(&self.raw_attestation)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::IntoSlotNumber;

    #[test]
    fn test_attestation_serialization() {
        let attestation = Attestation {
            initial_state_root: [3; 32],
            slot_hash: [10; 32],
            post_state_root: [22; 32],
            proof_of_bond: ProofOfBond {
                claimed_slot_number: 1.to_slot_number(),
                proof: (),
            },
        };

        let serialized_attestation = SerializedAttestation::from_attestation(&attestation).unwrap();
        let deserialized_attestation = serialized_attestation.to_attestation().unwrap();

        assert_eq!(attestation, deserialized_attestation);
    }
}
