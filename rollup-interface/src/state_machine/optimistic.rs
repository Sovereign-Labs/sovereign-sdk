//! Utilities for building an optimistic state machine
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

use crate::zk::StateTransition;

/// A proof that the attester was bonded at the transition num `transition_num`.
/// For rollups using the `jmt`, this will be a `jmt::SparseMerkleProof`
#[derive(
    Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize, Default,
)]
pub struct ProofOfBond<StateProof: BorshSerialize> {
    /// The actual state proof that the attester was bonded
    pub proof: StateProof,
    /// The transition number for which the proof of bond applies
    pub transition_num: u64,
}

/// An attestation that a particular DA layer block transitioned the rollup state to some value
#[derive(
    Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize, Default,
)]
pub struct Attestation<StateProof: BorshSerialize> {
    /// The alleged state root before applying the contents of the da block
    pub initial_state_root: [u8; 32],
    /// The hash of the block in which the transition occurred
    pub da_block_hash: [u8; 32],
    /// The alleged post-state root
    pub post_state_root: [u8; 32],
    /// A proof that the attester was bonded at some point in time before the attestation is generated
    pub proof_of_bond: ProofOfBond<StateProof>,
}

/// The contents of a challenge to an attestation, which are contained as a public output of the proof
/// Generic over an address type and a validity condition
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct ChallengeContents<Address, VC> {
    /// The rollup address of the originator of this challenge
    pub challenger_address: Address,
    /// The state transition that was proven
    pub state_transition: StateTransition<VC, Address>,
}

#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, Serialize, Deserialize)]
/// This struct contains the challenge as a raw blob
pub struct Challenge<'a>(&'a [u8]);
