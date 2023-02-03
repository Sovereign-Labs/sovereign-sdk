use crate::maybestd::rc::Rc;
use borsh::{BorshDeserialize, BorshSerialize};

use crate::{
    core::traits::{BatchTrait, TransactionTrait},
    serial::{Decode, DecodeBorrowed, DeserializationError, Encode},
};

/// An address on the DA layer. Opaque to the StateTransitionFunction
type OpaqueAddress = Rc<Vec<u8>>;

// TODO(@preston-evans98): update spec with simplified API
pub trait StateTransitionFunction {
    type StateRoot;
    type ChainParams;
    type Transaction: TransactionTrait;
    /// A batch of transactions. Also known as a "block" in most systems: we use
    /// the term batch in this context to avoid ambiguity with DA layer blocks
    type Batch: BatchTrait<Transaction = Self::Transaction>;
    type Proof: Decode<Error = DeserializationError>;

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

#[derive(Debug, Clone, Copy, BorshSerialize, BorshDeserialize)]
pub enum ConsensusRole {
    Prover,
    Sequencer,
    ProverAndSequencer,
}

/// A key-value pair representing a change to the rollup state
#[derive(Debug, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct Event {
    pub key: EventKey,
    pub value: EventValue,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash, BorshSerialize, BorshDeserialize)]
pub struct EventKey(Rc<Vec<u8>>);

#[derive(Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct EventValue(Rc<Vec<u8>>);

#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct ConsensusSetUpdate<Address> {
    pub address: Address,
    pub new_role: Option<ConsensusRole>,
}

#[derive(Debug, PartialEq, Clone)]
pub enum ConsensusMessage<B, P> {
    Batch(B),
    Proof(P),
}

#[derive(Debug, PartialEq, Clone)]
pub enum ConsensusMessageDecodeError<BatchErr, ProofErr> {
    Batch(BatchErr),
    Proof(ProofErr),
    NoTag,
    InvalidTag { max_allowed: u8, got: u8 },
}

impl<'de, P: DecodeBorrowed<'de>, B: DecodeBorrowed<'de>> DecodeBorrowed<'de>
    for ConsensusMessage<B, P>
{
    type Error = ConsensusMessageDecodeError<B::Error, P::Error>;
    fn decode_from_slice(target: &'de [u8]) -> Result<Self, Self::Error> {
        Ok(
            match *target
                .iter()
                .next()
                .ok_or(ConsensusMessageDecodeError::NoTag)?
            {
                0 => Self::Batch(
                    B::decode_from_slice(&mut &target[1..])
                        .map_err(|e| ConsensusMessageDecodeError::Batch(e))?,
                ),
                1 => Self::Proof(
                    P::decode_from_slice(&mut &target[1..])
                        .map_err(|e| ConsensusMessageDecodeError::Proof(e))?,
                ),
                _ => Err(ConsensusMessageDecodeError::InvalidTag {
                    max_allowed: 1,
                    got: target[0],
                })?,
            },
        )
    }
}
