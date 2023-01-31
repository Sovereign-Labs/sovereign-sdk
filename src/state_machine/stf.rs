use bytes::Bytes;

use crate::{
    core::traits::{BatchTrait, TransactionTrait},
    serial::{Decode, DeserializationError},
};

/// An address on the DA layer. Opaque to the StateTransitionFunction
type OpaqueAddress = Bytes;

// TODO(@preston-evans98): update spec with simplified API
pub trait StateTransitionFunction {
    type StateRoot;
    type ChainParams;
    type Transaction: TransactionTrait;
    /// A batch of transactions. Also known as a "block" in most systems: we use
    /// the term batch in this context to avoid ambiguity with DA layer blocks
    type Batch: BatchTrait<Transaction = Self::Transaction>;
    type Proof: Decode;

    /// A proof that the sequencer has misbehaved. For example, this could be a merkle proof of a transaction
    /// with an invalid signature
    type MisbehaviorProof;

    fn init_chain(&mut self, params: Self::ChainParams);

    /// Called at the beginning of each DA-layer block - whether or not that block contains any
    /// data relevant to the rollup.
    fn begin_slot(&self) -> StateUpdate;

    /// Apply a batch of transactions to the rollup, slashing the sequencer who proposed the batch on failure
    fn apply_batch(
        &self,
        cache: &mut StateUpdate,
        batch: Self::Batch,
        sequencer: &[u8],
        misbehavior_hint: Option<Self::MisbehaviorProof>,
    ) -> Result<Vec<Vec<Event>>, ConsensusSetUpdate<OpaqueAddress>>;

    fn apply_proof(
        &self,
        cache: &mut StateUpdate,
        proof: Self::Proof,
        prover: &[u8],
    ) -> Result<(), ConsensusSetUpdate<OpaqueAddress>>;

    /// Called once at the *end* of each DA layer block (i.e. after all rollup batches and proofs have been processed)
    /// Commits state changes to the database
    fn end_slot(
        &mut self,
        cache: StateUpdate,
    ) -> (Self::StateRoot, Vec<ConsensusSetUpdate<OpaqueAddress>>);
}

// TODO(@bkolad): replace with first-read-last-write cache
pub struct StateUpdate {}

#[derive(Debug, Clone, Copy)]
pub enum ConsensusRole {
    Prover,
    Sequencer,
    ProverAndSequencer,
}

/// A key-value pair representing a change to the rollup state
pub struct Event {
    pub key: Bytes,
    pub value: Bytes,
}

#[derive(Debug, Clone)]
pub struct ConsensusSetUpdate<Address> {
    pub address: Address,
    pub new_role: Option<ConsensusRole>,
}

pub enum ConsensusMessage<B, P> {
    Batch(B),
    Proof(P),
}

impl<P: Decode, B: Decode> Decode for ConsensusMessage<B, P> {
    fn decode(target: &mut &[u8]) -> Result<Self, crate::serial::DeserializationError> {
        Ok(
            match *target
                .iter()
                .next()
                .ok_or(DeserializationError::DataTooShort {
                    expected: 1,
                    got: 0,
                })? {
                0 => Self::Batch(B::decode(&mut &target[1..])?),
                1 => Self::Proof(P::decode(&mut &target[1..])?),
                _ => Err(DeserializationError::InvalidTag {
                    max_allowed: 1,
                    got: target[0],
                })?,
            },
        )
    }
}
