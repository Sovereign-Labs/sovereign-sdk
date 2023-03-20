use std::{cell::RefCell, sync::atomic::AtomicUsize};

use crate::{
    da::DaLayerTrait,
    maybestd::rc::Rc,
    serial::{Decode, DecodeBorrowed, Encode},
    stf::{ConsensusSetUpdate, StateTransitionFunction},
};

use super::{crypto::hash::DefaultHash, traits::Witness};

/// A block header of the *logical* chain created by running a particular state transition
/// function over a particular DA application.
///
/// In our model, there is a one-to-one correspondence between blocks of the data availability chain
/// and blocks of the rollup.
pub struct RollupHeader<DaLayer: DaLayerTrait, App: StateTransitionFunction> {
    /// The hash of the DA layer block corresponding to this rollup block
    pub da_blockhash: DaLayer::Blockhash,
    /// A commitment to the set of allowed sequencers after executing this block
    pub sequencers_root: ConsensusParticipantRoot<DaLayer::Address>,
    /// A commitment to the set of allowed provers after executing this block
    pub provers_root: ConsensusParticipantRoot<DaLayer::Address>,
    /// The state root of the main state transition function after executing this block
    pub app_root: App::StateRoot,
    /// A commitment to the set of da layer transactions that were actually applied to the rollup.
    /// A transaction is applied if it is "relevant" (see [relevant_txs](AugmentedRollupBlock) and its sender is in the
    /// set of allowed participants (i.e. a sequencer if it's a block, or a prover if it's a proof).
    pub applied_txs_root: DefaultHash,
    /// The hash of the previous rollup block header
    pub prev_hash: DefaultHash,
}

impl<D: DaLayerTrait, A: StateTransitionFunction> Encode for RollupHeader<D, A> {
    fn encode(&self, _target: &mut impl std::io::Write) {
        todo!()
    }
}

impl<D: DaLayerTrait, A: StateTransitionFunction> Decode for RollupHeader<D, A> {
    type Error = ();

    fn decode<R: std::io::Read>(_target: &mut R) -> Result<Self, <Self as Decode>::Error> {
        todo!()
    }
}

impl<'de, D: DaLayerTrait, A: StateTransitionFunction> DecodeBorrowed<'de> for RollupHeader<D, A> {
    type Error = ();

    fn decode_from_slice(_target: &'de [u8]) -> Result<Self, Self::Error> {
        todo!()
    }
}

impl<DaLayer: DaLayerTrait, App: StateTransitionFunction> RollupHeader<DaLayer, App> {
    pub fn hash(&self) -> DefaultHash {
        todo!()
    }
}

/// A block of the *logical* chain created by running a particular state transition
/// function over a particular DA application. A rollup block contains all of the information
/// needed to re-execute a state transition, but does not contain the auxiliary information
/// that would be needed to verify the fork-choice rule.
pub struct RollupBlock<DaLayer: DaLayerTrait, App: StateTransitionFunction> {
    /// The header of the logical rollup block
    pub header: RollupHeader<DaLayer, App>,
    /// The set of allowed sequencers after this block was processed
    pub sequencers: Vec<DaLayer::Address>,
    /// The set of allowed provers after this block was processed
    pub provers: Vec<DaLayer::Address>,
    /// The list of transactions applied
    pub applied_txs: Vec<DaLayer::BlobTransaction>,
}

/// A block of the *logical* chain, augmented with all of the information necessary
/// to execute the rollup's fork choice rule.
pub struct AugmentedRollupBlock<DaLayer: DaLayerTrait, App: StateTransitionFunction> {
    /// The state-transition information
    pub block: RollupBlock<DaLayer, App>,
    /// The header of the Da layer block corresponding to this rollup block
    pub da_header: DaLayer::BlockHeader,
    /// A list of transactions on the DA layer that are "relevant" to this rollup.
    /// A transaction is deemed "relevant" if its "sender" field needs to be examined
    /// in order to determine whether it applies to the rollup. For example, all transactions
    /// in a given Celestia namespace would be "relevant" to a rollup over that namespace.
    pub relevant_txs: Vec<DaLayer::BlobTransaction>,
    /// A witness showing that all of the relevant_txs were included in the DA block
    pub tx_witnesses: DaLayer::InclusionMultiProof,
    /// An additional witness showing that the list of relevant_txs is complete
    pub completeness_proof: DaLayer::CompletenessProof,
}

#[derive(Debug, Clone)]
pub enum ConsensusParticipantRoot<Addr> {
    /// Anyone is allowed to participate in consensus of the rollup
    Anyone,
    /// Only one centralized entity is allowed to participate
    Centralized(Addr),
    /// The set of allowed participants is registered. It may or may not change over time
    Registered(DefaultHash),
}

impl<Addr: PartialEq> ConsensusParticipantRoot<Addr> {
    pub fn allows(&self, participant: Addr) -> bool {
        match self {
            ConsensusParticipantRoot::Anyone => true,
            ConsensusParticipantRoot::Centralized(allowed_addr) => &participant == allowed_addr,
            ConsensusParticipantRoot::Registered(_) => todo!(),
        }
    }

    pub fn process_update(&mut self, _updates: ConsensusSetUpdate<Rc<Vec<u8>>>) {
        match self {
            ConsensusParticipantRoot::Anyone => todo!(),
            ConsensusParticipantRoot::Centralized(_) => todo!(),
            ConsensusParticipantRoot::Registered(_) => todo!(),
        }
    }
    pub fn process_updates(&mut self, _updates: Vec<ConsensusSetUpdate<Rc<Vec<u8>>>>) {
        // for item in vec.map(|| to_address).then(...)
        match self {
            ConsensusParticipantRoot::Anyone => todo!(),
            ConsensusParticipantRoot::Centralized(_) => todo!(),
            ConsensusParticipantRoot::Registered(_) => todo!(),
        }
    }

    pub fn finalize(&mut self) {
        todo!()
    }
}

#[derive(Default)]
pub struct ArrayWitness {
    next_idx: AtomicUsize,
    hints: RefCell<Vec<Vec<u8>>>,
}

impl Witness for ArrayWitness {
    fn add_hint<T: crate::serial::Encode + crate::serial::Decode>(&self, hint: T) {
        self.hints.borrow_mut().push(hint.encode_to_vec())
    }

    fn get_hint<T: crate::serial::Encode + crate::serial::Decode>(&self) -> T {
        let idx = self
            .next_idx
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        T::decode_from_slice(&self.hints.borrow()[idx]).unwrap()
    }

    fn merge(&self, rhs: &Self) {
        let rhs_next_idx = rhs.next_idx.load(std::sync::atomic::Ordering::SeqCst);
        self.hints
            .borrow_mut()
            .extend(rhs.hints.borrow_mut().drain(rhs_next_idx..))
    }
}
