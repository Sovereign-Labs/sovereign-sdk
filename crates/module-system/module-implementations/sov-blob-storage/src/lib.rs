#![deny(missing_docs)]
#![doc = include_str!("../README.md")]
mod capabilities;
mod max_size_checker;
mod validation;
use std::collections::BTreeMap;
use std::num::NonZero;
use std::sync::Arc;

use borsh::{BorshDeserialize, BorshSerialize};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sov_bank::derived_holder::DerivedHolder;
use sov_modules_api::macros::config_value;
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::{
    Amount, BatchWithId, BlobDataWithId, DaSpec, FullyBakedTx, GenesisState,
    InfallibleKernelStateAccessor, InjectedControlFlow, IterableBatchWithId, KernelStateAccessor,
    KernelStateMap, KernelStateValue, Module, ModuleId, ModuleInfo, NoOpControlFlow,
    NotInstantiable, PrivilegedKernelAccessor, SelectedBlob, Spec,
};
use sov_rollup_interface::common::SlotNumber;
use sov_state::codec::BcsCodec;

/// For how many slots deferred blobs are stored before being executed
pub fn config_deferred_slots_count() -> u64 {
    config_value!("DEFERRED_SLOTS_COUNT")
}

/// How many blobs from unregistered sequencers we will accept per slot
/// We can't slash misbehaving senders because they aren't a registered sequencer with a stake so
/// this serves as protection against spam.
pub fn config_unregistered_blobs_per_slot() -> u64 {
    config_value!("UNREGISTERED_BLOBS_PER_SLOT")
}

/// The type of sequencer that published a blob.
#[derive(
    Debug, PartialEq, Eq, Copy, Clone, BorshSerialize, BorshDeserialize, Serialize, Deserialize,
)]
pub enum SequencerType {
    /// The preferred sequencer with non-deferred execution privileges.
    Preferred,
    /// Any other sequencer, either registered with a standard registration or
    /// via emergency registration.
    NonPreferred,
}

/// An escrow account for storing the reserved gas for a blob.
#[derive(Clone, BorshSerialize, BorshDeserialize, Serialize, Deserialize, Debug, PartialEq)]
pub enum Escrow {
    /// The gas is held in a derived holder account in the bank module.
    DerivedHolder(DerivedHolder),
    /// The given number of tokens is held directly in the sequencer module.
    Direct(Amount),
    /// No gas is reserved.
    None,
}

/// A blob whose sender is allowed - either because he has sufficient balance or because it's one of the few
/// lucky "unregistered" sequencers who are being allowed to register this slot.
#[derive(Clone, BorshSerialize, BorshDeserialize, Serialize, Deserialize, Debug, PartialEq)]
#[serde(bound = "S: Spec, B: Serialize + DeserializeOwned")]
pub struct ValidatedBlob<S: Spec, B = IterableBatchWithId<S, NoOpControlFlow>> {
    /// Inner blob data.
    pub blob: BlobDataWithId<S, B>,
    sender: <<S as Spec>::Da as DaSpec>::Address,
    balance_store: Escrow,
}

impl<S: Spec> ValidatedBlob<S, BatchWithId<S>> {
    // TODO: Add proptests to confirm accuracy of this.
    pub(crate) fn conservative_serialized_size(
        blob: &BlobDataWithId<S, BatchWithId<S>>,
        sender: &<<S as Spec>::Da as DaSpec>::Address,
    ) -> usize {
        let mut size = blob.blob_size();
        size += 1; // for the blob enum discriminant
        size += sender.as_ref().len();
        size += 33; // For the option discriminant and derived holder size
        size += 12; // Account for serializing the blob length, the length of the sender field, and the length of the seqeuncer address if it's a proof
        size
    }
}
impl<S: Spec> ValidatedBlob<S, BatchWithId<S>> {
    /// Converts a validated blob into a selected blob at the given gas price.
    ///
    /// # Panics
    /// Panics if the gas calculation overflows. This is assumed to have been checked already.
    pub fn into_selected_blob<CF: InjectedControlFlow<S>>(
        self,
        cf: CF,
    ) -> SelectedBlob<S, IterableBatchWithId<S, CF>> {
        let gas_tokens = match self.balance_store {
            Escrow::DerivedHolder(_) => panic!("A blob reached the end of selection with its funds still in a deferred escrow. This is a bug!"),
            Escrow::Direct(amount) => Some(amount),
            Escrow::None => {
                assert!(self.blob.is_emergency_registration(), "Blob from known sender does not have reserved balance. This is a bug!");
                None
            },
        };
        SelectedBlob {
            blob_data: self.blob.map_batch(|b| IterableBatchWithId::new(b, cf)),
            sender: self.sender,
            reserved_gas_tokens: gas_tokens,
        }
    }
}
impl<S: Spec, B: Serialize + DeserializeOwned> ValidatedBlob<S, B> {
    /// Create a new validated blob.
    pub fn new(
        blob: BlobDataWithId<S, B>,
        sender: <<S as Spec>::Da as DaSpec>::Address,
        balance_store: Escrow,
    ) -> Self {
        Self {
            blob,
            sender,
            balance_store,
        }
    }
}

/// The sequence number for a batch from the preferred sequencer.   
pub type SequenceNumber = u64;

#[derive(
    Clone,
    Copy,
    BorshSerialize,
    BorshDeserialize,
    Serialize,
    Deserialize,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
)]
pub(crate) enum BlobType {
    Batch,
    Proof,
}

impl BlobType {
    pub fn is_batch(&self) -> bool {
        matches!(self, BlobType::Batch)
    }
}

/// Tracks the sequence numbers of the preferred sequencer.
#[derive(
    Clone, BorshSerialize, BorshDeserialize, Serialize, Deserialize, Debug, PartialEq, Default,
)]
pub struct SequencerNumberTracker {
    /// The next sequence number that we expect to see.
    next_sequence_number: SequenceNumber,
    /// A map of sequence numbers to the type of blob they correspond to. This lets us
    /// quickly decide whether we have a valid sequence of blobs to produce a rollup block without paying
    /// to load the entire set of blobs
    saved_sequencer_numbers: BTreeMap<SequenceNumber, BlobType>,
}

/// Blob storage contains only address and vector of blobs
#[derive(Clone, ModuleInfo)]
pub struct BlobStorage<S: Spec> {
    /// The ID of blob storage module
    #[id]
    pub(crate) id: ModuleId,

    /// Actual storage of blobs
    /// DA block number => vector of blobs
    /// Caller controls the order of blobs in the vector
    #[state]
    #[allow(clippy::type_complexity)]
    deferred_blobs: KernelStateMap<SlotNumber, Vec<ValidatedBlob<S, BatchWithId<S>>>, BcsCodec>,

    /// Any preferred sequencer blobs which were received out of order. Mapped from sequence number to batch.
    #[state]
    pub(crate) deferred_preferred_sequencer_blobs:
        KernelStateMap<SequenceNumber, PreferredBlobDataWithId>,

    /// A tracker for the upcoming sequence numbers for the preferred sequencer.
    #[state]
    pub(crate) upcoming_sequence_numbers: KernelStateValue<SequencerNumberTracker>,

    #[module]
    pub(crate) sequencer_registry: sov_sequencer_registry::SequencerRegistry<S>,

    #[module]
    chain_state: sov_chain_state::ChainState<S>,

    #[module]
    bank: sov_bank::Bank<S>,
}

/// Non standard methods for blob storage
impl<S: Spec> BlobStorage<S> {
    /// Store blobs for given block number, overwrite if already exists
    pub(crate) fn store_batches(
        &mut self,
        batches: &[ValidatedBlob<S, BatchWithId<S>>],
        state: &mut KernelStateAccessor<'_, S>,
    ) {
        // Optimization: we don't store any data for slots with no deferred
        // blobs.
        if !batches.is_empty() {
            // For the kernel state accessor, the `max_allowed_slot_number_to_access` is the true slot number
            self.deferred_blobs
                .set(&state.true_slot_number(), batches, state)
                .unwrap_infallible();
        }
    }

    /// Take all blobs for given block number, return empty vector if not exists
    /// Returned blobs are removed from the storage
    pub(crate) fn take_blobs_for_slot(
        &mut self,
        slot_number: SlotNumber,
        state: &mut impl InfallibleKernelStateAccessor,
    ) -> Vec<ValidatedBlob<S, BatchWithId<S>>> {
        self.deferred_blobs
            .remove(&slot_number, state)
            .unwrap_infallible()
            .unwrap_or_default()
    }

    pub(crate) fn get_preferred_sequencer(
        &self,
        state: &mut KernelStateAccessor<'_, S>,
    ) -> Option<(<<S as Spec>::Da as DaSpec>::Address, <S as Spec>::Address)> {
        self.sequencer_registry
            .get_preferred_sequencer(state)
            .unwrap_infallible()
    }

    /// Get the blob with the given sequence number, if it's saved.
    #[cfg(feature = "test-utils")]
    pub fn get_deferred_preferred_sequencer_blob<
        R: sov_modules_api::StateReader<sov_state::Kernel>,
    >(
        &self,
        sequence_number: u64,
        state: &mut R,
    ) -> Result<Option<PreferredBlobDataWithId>, R::Error> {
        self.deferred_preferred_sequencer_blobs
            .get(&sequence_number, state)
    }

    /// What the [`SequenceNumber`] of the next [`PreferredBlobData`]s MUST
    /// be.
    pub fn next_sequence_number(&self, state: &mut impl InfallibleKernelStateAccessor) -> u64 {
        self.upcoming_sequence_numbers
            .get(state)
            .unwrap_infallible()
            .map(|tracker| tracker.next_sequence_number)
            // The very first sequence number is always 0.
            .unwrap_or_default()
    }
}

/// Empty module implementation
impl<S: Spec> Module for BlobStorage<S> {
    type Spec = S;
    type Config = ();
    type CallMessage = NotInstantiable;
    type Event = ();

    fn genesis(
        &mut self,
        _genesis_rollup_header: &<<S as Spec>::Da as DaSpec>::BlockHeader,
        _config: &Self::Config,
        _state: &mut impl GenesisState<S>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn call(
        &mut self,
        _message: Self::CallMessage,
        _context: &sov_modules_api::Context<Self::Spec>,
        _state: &mut impl sov_modules_api::TxState<Self::Spec>,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}

/// Contains data obtained from the DA blob, plus metadata required for blobs
/// from the preferred sequencer. This is deserialized directly from the DA layer.
#[derive(Debug, PartialEq, Eq, Clone, BorshDeserialize, BorshSerialize, Serialize, Deserialize)]
pub struct PreferredBatchData {
    /// The sequence number of the batch/proof. The rollup attempts to process items in order by sequence number.
    /// For example, if the sequencer sends a batch with sequence number 2 followed by a proof with sequencer number 1,
    /// the rollup will defer processsing of the batch until the proof is received.
    pub sequence_number: u64,
    /// The actual data of the blob.
    pub data: Arc<Vec<FullyBakedTx>>,
    /// The number of visible slots to advance after processing the batch. Minimum 1.
    pub visible_slots_to_advance: NonZero<u8>,
}

/// A trait implemented by blobs sent through the preferred sequencer.
///
/// This allows the rollup to process them in order, even if they are
/// subsequently reordered by the DA layer.
pub trait PreferredSequenced: Into<PreferredBlobData> {
    /// The monotonic sequence number of the blob. The sequence number is shared
    /// across data types (so ordering is enforced between proofs and batches).
    fn sequence_number(&self) -> SequenceNumber;
}

impl PreferredSequenced for PreferredBatchData {
    fn sequence_number(&self) -> SequenceNumber {
        self.sequence_number
    }
}

/// Contains data obtained from the DA blob, plus metadata required for blobs
/// from the preferred sequencer. This is deserialized directly from the DA layer.
#[derive(Debug, PartialEq, Eq, Clone, BorshDeserialize, BorshSerialize, Serialize, Deserialize)]
pub struct PreferredProofData {
    /// The sequence number of the batch/proof. The rollup attempts to process items in order by sequence number.
    /// For example, if the sequencer sends a batch with sequence number 2 followed by a proof with sequencer number 1,
    /// the rollup will defer processsing of the batch until the proof is received.
    pub sequence_number: u64,
    /// The actual data of the blob.
    pub data: Vec<u8>,
}

impl PreferredSequenced for PreferredProofData {
    fn sequence_number(&self) -> SequenceNumber {
        self.sequence_number
    }
}

/// A preferred blob and the ID (hash) of the blob that it was deserialized from.
#[derive(Debug, PartialEq, Clone, BorshDeserialize, BorshSerialize, Serialize, Deserialize)]
pub struct PreferredBlobDataWithId {
    /// Raw transactions.
    pub inner: PreferredBlobData,
    /// The ID of the batch, carried over from the DA layer. This is the hash of the blob which contained the batch.
    pub id: [u8; 32],
}

/// The contents of a blob from the preferred sequencer, with the ID of the blob that it was deserialized from.
#[derive(
    Debug,
    PartialEq,
    Eq,
    Clone,
    BorshDeserialize,
    BorshSerialize,
    Serialize,
    Deserialize,
    derive_more::From,
)]
#[serde(rename_all = "snake_case")]
pub enum PreferredBlobData {
    /// A preferred blob from the batch namespace.
    Batch(PreferredBatchData),
    /// A preferred blob from the proof namespace.
    Proof(PreferredProofData),
}

impl PreferredBlobData {
    /// Returns the sequence number of the blob.
    pub fn sequence_number(&self) -> u64 {
        match self {
            PreferredBlobData::Batch(b) => b.sequence_number,
            PreferredBlobData::Proof(p) => p.sequence_number,
        }
    }

    /// Returns true if the blob is a batch.
    pub fn is_batch(&self) -> bool {
        matches!(self, PreferredBlobData::Batch(_))
    }

    /// Returns the number of visible slots to advance after processing the blob if it's a batch.
    pub fn visible_slot_number_increase(&self) -> Option<u8> {
        match self {
            PreferredBlobData::Batch(b) => Some(b.visible_slots_to_advance.get()),
            PreferredBlobData::Proof(_) => None,
        }
    }
}
