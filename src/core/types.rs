use bytes::Bytes;

use crate::{
    da::DaApp,
    stf::{ConsensusSetUpdate, StateTransitionFunction},
    zk_utils::traits::serial::{Deser, Serialize},
};

use super::crypto::hash::DefaultHash;

/// A block header of the *logical* chain created by running a particular state transition
/// function over a particular DA application.
///
/// In our model, there is a one-to-one correspondence between blocks of the data availability chain
/// and blocks of the rollup.
pub struct RollupHeader<DaLayer: DaApp, App: StateTransitionFunction> {
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

impl<DaLayer: DaApp, App: StateTransitionFunction> RollupHeader<DaLayer, App> {
    pub fn hash(&self) -> DefaultHash {
        todo!()
    }
}

impl<DaLayer: DaApp, App: StateTransitionFunction> Serialize for RollupHeader<DaLayer, App> {
    fn serialize(&self, target: &mut Vec<u8>) {
        todo!()
    }
}
impl<DaLayer: DaApp, App: StateTransitionFunction> Deser for RollupHeader<DaLayer, App> {
    fn deser(
        target: &mut &[u8],
    ) -> Result<Self, crate::zk_utils::traits::serial::DeserializationError> {
        todo!()
    }
}

/// A block of the *logical* chain created by running a particular state transition
/// function over a particular DA application. A rollup block contains all of the information
/// needed to re-execute a state transition, but does not contain the auxiliary information
/// that would be needed to verify the fork-choice rule.
pub struct RollupBlock<DaLayer: DaApp, App: StateTransitionFunction> {
    /// The header of the logical rollup block
    pub header: RollupHeader<DaLayer, App>,
    /// The set of allowed sequencers after this block was processed
    pub sequencers: Vec<DaLayer::Address>,
    /// The set of allowed provers after this block was processed
    pub provers: Vec<DaLayer::Address>,
    /// The list of transactions applied
    pub applied_txs: Vec<DaLayer::Transaction>,
}

/// A block of the *logical* chain, augmented with all of the information necessary
/// to execute the rollup's fork choice rule.
pub struct AugmentedRollupBlock<DaLayer: DaApp, App: StateTransitionFunction> {
    /// The state-transition information
    pub block: RollupBlock<DaLayer, App>,
    /// The header of the Da layer block corresponding to this rollup block
    pub da_header: DaLayer::Header,
    /// A list of transactions on the DA layer that are "relevant" to this rollup.
    /// A transaction is deemed "relevant" if its "sender" field needs to be examined
    /// in order to determine whether it applies to the rollup. For example, all transactions
    /// in a given Celestia namespace would be "relevant" to a rollup over that namespace.
    pub relevant_txs: Vec<DaLayer::Transaction>,
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

    pub fn process_update(&mut self, _updates: ConsensusSetUpdate<Bytes>) {
        match self {
            ConsensusParticipantRoot::Anyone => todo!(),
            ConsensusParticipantRoot::Centralized(_) => todo!(),
            ConsensusParticipantRoot::Registered(_) => todo!(),
        }
    }
    pub fn process_updates(&mut self, _updates: Vec<ConsensusSetUpdate<Bytes>>) {
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
