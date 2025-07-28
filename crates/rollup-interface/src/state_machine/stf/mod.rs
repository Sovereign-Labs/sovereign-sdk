//! This module is the core of the Sovereign SDK. It defines the traits and types that
//! allow the SDK to run the "business logic" of any application generically.
//!
//! The most important trait in this module is the [`StateTransitionFunction`], which defines the
//! main event loop of the rollup.

mod events;
#[cfg(any(test, feature = "arbitrary"))]
pub mod fuzzing;
mod proof_sender;
mod transaction;
mod verifier;

use std::fmt::{Debug, Display};

pub use events::*;
pub use proof_sender::*;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use thiserror::Error;
pub use transaction::*;
pub use verifier::StateTransitionVerifier;

use super::optimistic::Attestation;
use crate::common::{HexHash, RollupHeight};
use crate::da::{DaSpec, RelevantBlobIters};
use crate::zk::aggregated_proof::{AggregatedProofPublicData, SerializedAggregatedProof};
use crate::zk::{StateTransitionPublicData, Zkvm};

/// The configuration of a full node of the rollup which creates zk proofs.
pub struct ProverConfig;
/// The configuration used to initialize the "Verifier" of the state transition function
/// which runs inside of the zkVM.
pub struct ZkConfig;
/// The configuration of a standard full node of the rollup which does not create zk proofs
pub struct StandardConfig;

/// A special marker trait which allows us to define different rollup configurations. There are
/// only 3 possible instantiations of this trait: [`ProverConfig`], [`ZkConfig`], and [`StandardConfig`].
pub trait StateTransitionConfig: sealed::Sealed {}
impl StateTransitionConfig for ProverConfig {}
impl StateTransitionConfig for ZkConfig {}
impl StateTransitionConfig for StandardConfig {}

// https://rust-lang.github.io/api-guidelines/future-proofing.html
mod sealed {
    use super::{ProverConfig, StandardConfig, ZkConfig};

    pub trait Sealed {}
    impl Sealed for ProverConfig {}
    impl Sealed for ZkConfig {}
    impl Sealed for StandardConfig {}
}

/// A receipt for a batch of transactions. These receipts are stored in the rollup's database
/// and may be queried via RPC. Batch receipts are generic over a type `BatchReceiptContents` which the rollup
/// can use to store arbitrary typed data, like the gas used by the batch. They are also generic over a type `TxReceiptContents`,
/// since they contain a vectors of [`TransactionReceipt`]s.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound = "T: TxReceiptContents, BatchReceiptContents: Serialize + DeserializeOwned + Clone")]
pub struct BatchReceipt<BatchReceiptContents, T: TxReceiptContents> {
    /// The canonical hash of this batch
    pub batch_hash: [u8; 32],
    /// The receipts of all executed transactions in this batch.
    pub tx_receipts: Vec<TransactionReceipt<T>>,
    /// The receipts of all ignored transactions in this batch.
    pub ignored_tx_receipts: Vec<IgnoredTransactionReceipt<T>>,
    /// Any additional structured data to be saved in the database and served over RPC
    pub inner: BatchReceiptContents,
}

/// A receipt for data posted into the proof namespace
#[derive(Debug, Clone)]
pub struct ProofReceipt<Address, Da: DaSpec, Root, StorageProof> {
    /// The hash of the blob which contained the proof
    pub blob_hash: [u8; 32],
    /// The outcome of the proof
    pub outcome: ProofOutcome<Address, Da, Root, StorageProof>,
    /// Total gas incurred for this proof. This does not include the priority fee.
    pub gas_used: Vec<u64>,
    /// Computed gas price for this proof.
    pub gas_price: Vec<u128>,
}

/// The contents of a proof receipt.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum ProofReceiptContents<Address, Da: DaSpec, Root, StorageProof> {
    /// A receipt for an aggregate proof contains the public data form the proof and the serialized proof.
    AggregateProof(
        AggregatedProofPublicData<Address, Da, Root>,
        SerializedAggregatedProof,
    ),
    /// A receipt for a block proof contains the public data from the state transition which was proven.
    BlockProof(StateTransitionPublicData<Address, Da, Root>),
    /// A receipt for an attestation contains the public data that the attestation made a claim about.
    Attestation(Attestation<Da::SlotHash, Root, StorageProof>),
}

/// The context in which the execution is happening.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExecutionContext {
    /// The transaction is being executed by a sequencer before inclusion.
    Sequencer,
    /// The transaction is being executed by a node after inclusion.
    Node,
}

impl ExecutionContext {
    /// Returns true if and only if `self` matches [`ExecutionContext::Sequencer`].
    pub fn is_sequencer(&self) -> bool {
        match self {
            Self::Sequencer => true,
            Self::Node => false,
        }
    }
}

/// The error returned when the proof that was processed is invalid.
#[derive(Debug, Clone, Error, Eq, PartialEq)]
pub enum InvalidProofError {
    /// A precondition for processing the proof was not met.
    #[error("A precondition required to process the proof was not met: {0}")]
    PreconditionNotMet(String),
    /// The prover was slashed for invalid proof.
    #[error("Prover was slashed: {0}")]
    ProverSlashed(String),
    /// The prover was penalized.
    #[error("Prover was penalized: {0}")]
    ProverPenalized(String),
    /// Failed to reward the submitter of the proof.
    #[error("Failed to reward submitter: {0}. Rewarding module might not have enough funds. This is a bug!")]
    RewardFailure(String),
    /// An error occurred when accessing the state
    #[error("Error occurred when accessing the state, error: {0}")]
    StateAccess(String),
}

impl InvalidProofError {
    /// Checks if the error is revertable.
    pub fn is_not_revertable(&self) -> bool {
        matches!(
            self,
            InvalidProofError::ProverSlashed(_) | InvalidProofError::ProverPenalized(_)
        )
    }
}

/// The outcome of a proof
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum ProofOutcome<Address, Da: DaSpec, Root, StorageProof> {
    /// The blob was filtered out as irrelevant
    Ignored,
    /// The blob is some kind of valid proof
    Valid(ProofReceiptContents<Address, Da, Root, StorageProof>),
    /// The blob is some kind of invalid proof
    Invalid(InvalidProofError),
}

type ProofReceipts<Address, Da, StateRoot, StorageProof> =
    Vec<ProofReceipt<Address, Da, StateRoot, StorageProof>>;

/// The result of applying a slot to current state. We define a helpful alias [`ApplySlotOutput`]
/// to make the type signature of [`StateTransitionFunction::apply_slot`] more readable. Unfortunately,
/// since this type both depends on and appears in the defintion of the [`StateTransitionFunction`] trait,
/// we have to use a type alias to avoid introducing an unneeded [`Sized`] bound.
pub struct ApplySlotOutputInner<Root, ChangeSet, BR, PR, Witness> {
    /// Final state root after all blobs were applied
    pub state_root: Root,
    /// Container for all state alterations that happened during slot execution
    pub change_set: ChangeSet,
    /// Receipt for each applied proof transaction
    pub proof_receipts: PR,
    /// Receipt for each applied batch
    pub batch_receipts: Vec<BR>,
    /// Hash for each discarded blob.
    pub discarded_blobs: Vec<HexHash>,
    /// Witness after applying the whole block
    pub witness: Witness,
    /// The rollup height before applying the changes.
    pub rollup_height: RollupHeight,
}

/// The result of applying a slot to current state.
#[allow(type_alias_bounds)]
pub type ApplySlotOutput<
    InnerVm: Zkvm,
    OuterVm: Zkvm,
    Da: DaSpec,
    Stf: StateTransitionFunction<InnerVm, OuterVm, Da>,
> = ApplySlotOutputInner<
    Stf::StateRoot,
    Stf::ChangeSet,
    BatchReceipt<Stf::BatchReceiptContents, Stf::TxReceiptContents>,
    ProofReceipts<Stf::Address, Da, Stf::StateRoot, Stf::StorageProof>,
    Stf::Witness,
>;

// TODO(@preston-evans98): update spec with simplified API
/// State transition function defines business logic that responsible for changing state.
/// Terminology:
///  - state root: root hash of state merkle tree
///  - block: DA layer block
///  - batch: Set of transactions grouped together, or block on L2
///  - blob: Non serialised batch or anything else that can be posted on DA layer, like attestation or proof.
///
/// The STF is generic over a DA layer and two `Zkvm`s. The `InnerVm` is used to prove individual slots,
/// while the `OuterVm` is used to generate recursive proofs over multiple slots. The two VMs *may* be set to be
/// the  same type.
pub trait StateTransitionFunction<InnerVm: Zkvm, OuterVm: Zkvm, Da: DaSpec> {
    /// Root hash of state merkle tree
    type StateRoot: Serialize
        + DeserializeOwned
        + Clone
        + AsRef<[u8]>
        + Debug
        + Display
        + Send
        + Sync
        + 'static;

    /// The address of the prover.
    type Address: Serialize + DeserializeOwned + Clone + Debug;

    /// The initial params of the rollup.
    type GenesisParams;

    /// State of the rollup before transition.
    type PreState;

    /// State of the rollup after transition.
    type ChangeSet;

    /// Gas price type.
    type GasPrice: Debug;

    /// The storage proof for attestation.
    type StorageProof: Serialize + DeserializeOwned + Clone + Debug;

    /// The contents of a transaction for a successful transaction.
    type TxReceiptContents: TxReceiptContents;

    /// The contents of a batch receipt. This is the data that is persisted in the database
    type BatchReceiptContents: Serialize + DeserializeOwned + Clone + Send + Sync + Debug;

    /// Witness is a data that is produced during actual batch execution
    /// or validated together with proof during verification
    type Witness: Default + Serialize + DeserializeOwned + Send + Sync + 'static;

    /// Perform one-time initialization for the genesis block and
    /// returns the resulting root hash and changeset.
    /// If the init chain fails we panic.
    fn init_chain(
        &self,
        genesis_rollup_header: &Da::BlockHeader,
        genesis_state: Self::PreState,
        params: Self::GenesisParams,
    ) -> (Self::StateRoot, Self::ChangeSet);

    /// Called at each **DA-layer block** - whether or not that block contains any
    /// data relevant to the rollup.
    /// If slot is started in Full Node mode, default witness should be provided.
    /// If slot is started in Zero Knowledge mode, witness from execution should be provided.
    ///
    /// Applies batches of transactions to the rollup,
    /// slashing the sequencer who proposed the blob on failure.
    /// The blobs are contained into a slot whose data is contained within the `slot_data` parameter,
    /// this parameter is mainly used within the begin_slot hook.
    /// The concrete blob type is defined by the DA layer implementation,
    /// which is why we use a generic here instead of an associated type.
    ///
    /// Commits state changes to the database
    #[allow(clippy::type_complexity, clippy::too_many_arguments)]
    fn apply_slot(
        &self,
        pre_state_root: &Self::StateRoot,
        pre_state: Self::PreState,
        witness: Self::Witness,
        slot_header: &Da::BlockHeader,
        relevant_blobs: RelevantBlobIters<&mut [<Da as DaSpec>::BlobTransaction]>,
        execution_context: ExecutionContext,
    ) -> ApplySlotOutput<InnerVm, OuterVm, Da, Self>;
}
