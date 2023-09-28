//! Utilities for building an optimistic state machine
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

use crate::da::DaSpec;
use crate::zk::StateTransition;

/// A proof that the attester was bonded at the transition num `transition_num`.
/// For rollups using the `jmt`, this will be a `jmt::SparseMerkleProof`
#[derive(
    Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize, Default,
)]
pub struct ProofOfBond<StateProof> {
    /// The transition number for which the proof of bond applies
    pub claimed_transition_num: u64,
    /// The actual state proof that the attester was bonded
    pub proof: StateProof,
}

/// An attestation that a particular DA layer block transitioned the rollup state to some value
#[derive(
    Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize, Default,
)]
pub struct Attestation<Da: DaSpec, StateProof, StateRoot> {
    /// The alleged state root before applying the contents of the da block
    pub initial_state_root: StateRoot,
    /// The hash of the block in which the transition occurred
    pub da_block_hash: Da::SlotHash,
    /// The alleged post-state root
    pub post_state_root: StateRoot,
    /// A proof that the attester was bonded at some point in time before the attestation is generated
    pub proof_of_bond: ProofOfBond<StateProof>,
}

/// The contents of a challenge to an attestation, which are contained as a public output of the proof
/// Generic over an address type and a validity condition
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct ChallengeContents<Address, Da: DaSpec, Root> {
    /// The rollup address of the originator of this challenge
    pub challenger_address: Address,
    /// The state transition that was proven
    pub state_transition: StateTransition<Da, Address, Root>,
}

#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, Serialize, Deserialize)]
/// This struct contains the challenge as a raw blob
pub struct Challenge<'a>(&'a [u8]);
