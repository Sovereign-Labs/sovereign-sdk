use sov_rollup_interface::common::{HexHash, SlotNumber};
use sov_rollup_interface::da::{BlobReaderTrait, DaSpec, RelevantBlobIters};

use crate::{
    as_u32_or_panic, InjectedControlFlow, IterableBatchWithId, KernelStateAccessor, SelectedBlob,
    Spec,
};

/// The namespace in which a blob appeared.
#[derive(Debug, PartialEq, Eq, Hash)]
pub enum BlobOrigin<'a, T> {
    /// The blob is from the "batch" namespace. These blobs contain transactions.
    Batch(&'a mut T),
    /// The blob is from the "proof" namespace. These blobs contain proofs.
    Proof(&'a mut T),
}

impl<T: BlobReaderTrait> BlobOrigin<'_, T> {
    /// Inner blob data.
    pub fn get(&self) -> &T {
        match self {
            BlobOrigin::Batch(b) => b,
            BlobOrigin::Proof(p) => p,
        }
    }

    /// Returns the total number of bytes in the blob.
    #[must_use]
    pub fn total_len(&self) -> u32 {
        match self {
            BlobOrigin::Batch(b) => as_u32_or_panic(b.total_len()),
            BlobOrigin::Proof(p) => as_u32_or_panic(p.total_len()),
        }
    }
}

/// Output of the [`BlobSelector::get_blobs_for_this_slot`] method from the [`BlobSelector`] trait.
pub struct BlobSelectorOutput<ValidatedBlobType> {
    /// The blobs selected by the module.
    pub selected_blobs: Vec<ValidatedBlobType>,
    /// By how much the visible slot number should be increased.
    ///
    /// When greater than zero, a new rollup block will be created. If this value is zero,
    /// the list of selected blobs must also be empty. This is enforced by the rollup blueprint.
    pub visible_slot_number_increase: u64,
}

impl<B> BlobSelectorOutput<B> {
    /// Whether the rollup block hooks should be executed. We should execute block hooks whenever we accept blobs for execution
    /// or when we increase the visible slot number.
    pub fn creates_rollup_block(&self) -> bool {
        self.visible_slot_number_increase > 0
    }

    /// Apply the given function to each blob
    pub fn map_blobs<Target>(self, f: impl FnMut(B) -> Target) -> BlobSelectorOutput<Target> {
        BlobSelectorOutput {
            selected_blobs: self.selected_blobs.into_iter().map(f).collect(),
            visible_slot_number_increase: self.visible_slot_number_increase,
        }
    }
}

/// `BlobSelector` decides which blobs to process in a current slot.
pub trait BlobSelector {
    /// Spec type
    type Spec: Spec;

    /// Whether the kernel accepts "preferred" batches in a special format.
    const ACCEPTS_PREFERRED_BATCHES: bool;

    /// Returns a vector of blobs that should be processed in the current slot.
    ///
    /// The blob selector is responsible for crucial security properties of the rollup. It should...
    /// - Limit the number of "emergency registration" blobs that it accepts to a sensible number
    /// - Ensure that the total amount of blobs stored in the blob selector is not too large
    /// - Ensure that no blob is read without being paid for unless there is a very good reason (i.e. a small number of emergency registrations per slot)
    /// - Ensure that no blobs are selected for execution without a corresponding virtual height increase
    /// - Ensure that the preferred sequencer can't censor blobs by consuming all available rollup-block space.
    #[allow(clippy::type_complexity)]
    fn get_blobs_for_this_slot<CF: InjectedControlFlow<Self::Spec> + Clone>(
        &mut self,
        current_blobs: RelevantBlobIters<
            &mut [<<Self::Spec as Spec>::Da as DaSpec>::BlobTransaction],
        >,
        state: &mut KernelStateAccessor<'_, Self::Spec>,
        cf: CF,
    ) -> anyhow::Result<(
        BlobSelectorOutput<SelectedBlob<Self::Spec, IterableBatchWithId<Self::Spec, CF>>>,
        Vec<HexHash>,
    )>;

    /// Extracts all delayed non-preferred blobs that belong to the given slots.
    #[allow(clippy::type_complexity)]
    fn get_non_preferred_blobs<CF: InjectedControlFlow<Self::Spec> + Clone>(
        &mut self,
        slot_range: impl Iterator<Item = SlotNumber>,
        state: &mut KernelStateAccessor<'_, Self::Spec>,
        cf: CF,
    ) -> Vec<SelectedBlob<Self::Spec, IterableBatchWithId<Self::Spec, CF>>>;

    /// Add funds to the escrow for the preferred sequencer.
    #[cfg(feature = "native")]
    fn escrow_funds_for_preferred_sequencer(
        &mut self,
        amount: crate::Amount,
        state: &mut KernelStateAccessor<'_, Self::Spec>,
    ) -> anyhow::Result<()>;

    /// Implementors that don't support preferred blobs SHOULD panic.
    fn next_sequence_number(&self, _state: &mut KernelStateAccessor<'_, Self::Spec>) -> u64 {
        panic!("Kernel does not support preferred blobs. Please change kernel type.")
    }
}
