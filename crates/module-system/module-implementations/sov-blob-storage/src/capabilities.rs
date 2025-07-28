use std::sync::Arc;

use borsh::BorshDeserialize;
use sov_bank::derived_holder::DerivedHolder;
use sov_bank::IntoPayable;
use sov_modules_api::capabilities::{AllowedSequencer, BlobOrigin, BlobSelectorOutput};
use sov_modules_api::macros::config_value;
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::{
    as_u32_or_panic, Amount, BatchWithId, BlobData, BlobDataWithId, BlobReaderTrait, DaSpec,
    FullyBakedTx, Gas, GasArray, GasSpec, HexHash, HexString, InjectedControlFlow,
    IterableBatchWithId, KernelStateAccessor, ModuleInfo, PrivilegedKernelAccessor, SelectedBlob,
    Spec,
};
use sov_rollup_interface::common::SlotNumber;
use sov_rollup_interface::da::RelevantBlobIters;
use sov_sequencer_registry::AllowedSequencerError;
use tracing::{debug, info, trace, warn};

use crate::max_size_checker::{BlobsAccumulatorWithSizeLimit, PushOrIgnore};
use crate::{
    config_deferred_slots_count, config_unregistered_blobs_per_slot, BlobStorage, BlobType, Escrow,
    PreferredBatchData, PreferredBlobData, PreferredBlobDataWithId, PreferredProofData,
    SequenceNumber, SequencerNumberTracker, SequencerType, ValidatedBlob,
};
/// A loose upper bound on the size of an emergency registration blob, in bytes. Blobs larger than this are statically known to be invalid
/// so we don't bother trying to deserialize them.
const MAX_EMERGENCY_REGISTRATION_BLOB_SIZE: usize = 1000;

/// The reason that a blob was discarded
#[derive(Debug)]
pub(crate) enum BlobDiscardReason {
    /// The sequencer sent a blob with an old sequencer number that we've already processed.
    SequenceNumberTooLow,
    /// Sender doesn't have enough staked sequencer funds
    SenderInsufficientStake,
    /// The max amount of unregistered blobs allowed to be processed per slot
    MaxAllowedUnregisteredBlobs,
    /// The blob is too large to be processed with the remaining capacity to accept blobs this slot
    OutOfCapacity,
    /// The blob has insufficient reserved gas to cover the pre-execution checks. This happens when the gas price more
    /// than doubles while a blob is in storage.
    InsufficientReservedGas,
    /// The blob was not serialized correctly
    InvalidSerialization,
    /// The blob is too large to be processed to be a valid emergency registration.
    EmergencyRegistrationTooLarge,
}

#[derive(Debug)]
enum SequencerStatus<S: Spec> {
    Registered(AllowedSequencer<S>),
    Unregistered,
}

enum BlobArrival {
    New(PreferredBlobDataWithId),
    Stored(SequenceNumber, BlobType),
}

impl BlobArrival {
    fn blob_type(&self) -> BlobType {
        match self {
            BlobArrival::New(blob) => blob.inner.blob_type(),
            BlobArrival::Stored(_, blob_type) => *blob_type,
        }
    }

    #[cfg(test)]
    fn sequence_number(&self) -> u64 {
        match self {
            BlobArrival::New(blob) => blob.inner.sequence_number(),
            BlobArrival::Stored(sequence_number, _) => *sequence_number,
        }
    }
}
impl PreferredBlobData {
    fn blob_type(&self) -> BlobType {
        match self {
            PreferredBlobData::Proof(_) => BlobType::Proof,
            PreferredBlobData::Batch(_) => BlobType::Batch,
        }
    }
}
enum ValidateBlobOutcome<S: Spec> {
    Discard(BlobDiscardReason),
    Accept(SequencerStatus<S>),
}

struct SeparatedBlobs<'a, S: Spec> {
    preferred_blobs: Vec<&'a mut <S::Da as DaSpec>::BlobTransaction>,
    non_preferred_blobs: Vec<&'a mut <S::Da as DaSpec>::BlobTransaction>,
}

impl<S: Spec> BlobStorage<S> {
    /// Select the blobs to execute this slot using "based sequencing". In this mode,
    /// blobs are processed in the order that they appear on the DA layer.
    fn select_blobs_as_based_sequencer_inner(
        &mut self,
        current_blobs: RelevantBlobIters<&mut [<S::Da as DaSpec>::BlobTransaction]>,
        discarded_blobs: &mut Vec<HexHash>,
        state: &mut KernelStateAccessor<'_, S>,
    ) -> BlobSelectorOutput<ValidatedBlob<S, BatchWithId<S>>> {
        tracing::trace!("On based sequencer path");

        let visible_slot_number_increase = state
            .true_slot_number()
            .get()
            .saturating_sub(state.visible_slot_number().get());

        BlobSelectorOutput {
            selected_blobs: self.select_blobs_da_ordering(
                current_blobs,
                discarded_blobs,
                false,
                visible_slot_number_increase,
                state,
            ),
            visible_slot_number_increase,
        }
    }

    #[allow(clippy::type_complexity)]
    fn select_blobs_da_ordering(
        &mut self,
        current_blobs: RelevantBlobIters<&mut [<S::Da as DaSpec>::BlobTransaction]>,
        discarded_blobs: &mut Vec<HexHash>,
        account_for_deferral: bool,
        visible_height_increase: u64,
        state: &mut KernelStateAccessor<'_, S>,
    ) -> Vec<ValidatedBlob<S, BatchWithId<S>>> {
        let mut blobs_with_total_size_limit = BlobsAccumulatorWithSizeLimit::<S>::new();

        let current_blobs = current_blobs
            .batch_blobs
            .iter_mut()
            .map(BlobOrigin::Batch)
            .chain(current_blobs.proof_blobs.iter_mut().map(BlobOrigin::Proof));

        self.select_blobs_da_ordering_helper(
            current_blobs,
            &mut blobs_with_total_size_limit,
            discarded_blobs,
            account_for_deferral,
            visible_height_increase,
            state,
        );
        blobs_with_total_size_limit.inner()
    }

    // Selects blobs from the DA layer in the order they appear, subject to the given size limit. Note that this method
    // mutatates the `blobs_with_total_size_limit` argument rather than returning a new value - this makes it easy to share
    // beween the preferred and non-preferred paths.
    fn select_blobs_da_ordering_helper<'a>(
        &mut self,
        blob_iter: impl Iterator<Item = BlobOrigin<'a, <S::Da as DaSpec>::BlobTransaction>>,
        blobs_with_total_size_limit: &mut BlobsAccumulatorWithSizeLimit<S>,
        discarded_blobs: &mut Vec<HexHash>,
        account_for_deferral: bool,
        visible_height_increase: u64,
        state: &mut KernelStateAccessor<'_, S>,
    ) {
        let mut unregistered_blob_count = 0;
        let gas_price_for_new_block = self.get_new_gas_price(visible_height_increase, state);
        for (idx, item) in blob_iter.enumerate() {
            tracing::trace!(idx, "Processing blob");

            let sequencer_type = SequencerType::NonPreferred;
            if !blobs_with_total_size_limit.can_accept_blob(sequencer_type, item.get().total_len())
            {
                let blob = item.get();
                Self::discard(
                    discarded_blobs,
                    &blob.sender(),
                    blob.hash().into(),
                    &BlobDiscardReason::OutOfCapacity,
                );
                continue;
            }

            match item {
                BlobOrigin::Proof(blob) => {
                    tracing::trace!(idx, "Processing as proof");
                    let Some(proof) = self.try_validate_proof_and_reserve_funds(
                        as_u32_or_panic(idx),
                        blob,
                        account_for_deferral,
                        visible_height_increase,
                        state,
                    ) else {
                        Self::discard(
                            discarded_blobs,
                            &blob.sender(),
                            blob.hash().into(),
                            &BlobDiscardReason::SenderInsufficientStake,
                        );
                        continue;
                    };
                    if let PushOrIgnore::IgnoredBlob(id, sender) = blobs_with_total_size_limit
                        .push_or_ignore(SequencerType::NonPreferred, proof)
                    {
                        Self::discard(
                            discarded_blobs,
                            &sender,
                            id,
                            &BlobDiscardReason::OutOfCapacity,
                        );
                    }
                }
                BlobOrigin::Batch(blob) => {
                    tracing::trace!("Processing as batch");
                    match self.pre_validate_blob_and_sender(blob, unregistered_blob_count, state) {
                        ValidateBlobOutcome::Accept(SequencerStatus::Registered(sequencer)) => {
                            let Some(validated) = self
                                .try_validate_batch_and_reserve_funds_if_needed(
                                    as_u32_or_panic(idx),
                                    blob,
                                    sequencer,
                                    &gas_price_for_new_block,
                                    account_for_deferral,
                                    state,
                                )
                            else {
                                Self::discard(
                                    discarded_blobs,
                                    &blob.sender(),
                                    blob.hash().into(),
                                    &BlobDiscardReason::SenderInsufficientStake,
                                );
                                continue;
                            };
                            // TODO: If the preferred sequencer advanced the visible slot number too much, blobs will get dropped here.
                            // This could be used for censorship, but the attack is not economically feasible (analysis available upon request).
                            // We should consider trying to detect and slash for this.
                            if let PushOrIgnore::IgnoredBlob(id, sender) =
                                blobs_with_total_size_limit
                                    .push_or_ignore(SequencerType::NonPreferred, validated)
                            {
                                Self::discard(
                                    discarded_blobs,
                                    &sender,
                                    id,
                                    &BlobDiscardReason::OutOfCapacity,
                                );
                            }
                        }
                        ValidateBlobOutcome::Accept(SequencerStatus::Unregistered) => {
                            // If the blob is too large to be a valid emergency registration, just discard it.
                            if blob.total_len() > MAX_EMERGENCY_REGISTRATION_BLOB_SIZE {
                                Self::discard(
                                    discarded_blobs,
                                    &blob.sender(),
                                    blob.hash().into(),
                                    &BlobDiscardReason::EmergencyRegistrationTooLarge,
                                );
                                continue;
                            }
                            // Otherwise, try to deserialize and use it
                            unregistered_blob_count += 1;
                            if let Some(tx) = self.deserialize_or_try_slash_sender::<FullyBakedTx>(
                                blob, None, false, state,
                            ) {
                                let blob = ValidatedBlob::new(
                                    BlobData::EmergencyRegistration(tx).with_id(blob.hash().into()),
                                    blob.sender(),
                                    Escrow::None,
                                );
                                if let PushOrIgnore::IgnoredBlob(id, sender) =
                                    blobs_with_total_size_limit
                                        .push_or_ignore(SequencerType::NonPreferred, blob)
                                {
                                    BlobStorage::<S>::discard(
                                        discarded_blobs,
                                        &sender,
                                        id,
                                        &BlobDiscardReason::OutOfCapacity,
                                    );
                                }
                                continue;
                            }

                            Self::discard(
                                discarded_blobs,
                                &blob.sender(),
                                blob.hash().into(),
                                &BlobDiscardReason::InvalidSerialization,
                            );
                        }
                        ValidateBlobOutcome::Discard(reason) => {
                            Self::discard(
                                discarded_blobs,
                                &blob.sender(),
                                blob.hash().into(),
                                &reason,
                            );
                        }
                    }
                }
            }
        }
    }

    /// Check that the sequencer has enough stake to pay for blob deserialization.
    fn pre_validate_blob_and_sender(
        &self,
        blob: &<S::Da as DaSpec>::BlobTransaction,
        unregistered_blobs_processed: u64,
        state: &mut KernelStateAccessor<S>,
    ) -> ValidateBlobOutcome<S> {
        match self
            .sequencer_registry
            .is_sender_allowed(&blob.sender(), state)
        {
            Ok(sequencer) => ValidateBlobOutcome::Accept(SequencerStatus::Registered(sequencer)),
            Err(AllowedSequencerError::NotRegistered) | Err(AllowedSequencerError::NotActive) => {
                if unregistered_blobs_processed >= config_unregistered_blobs_per_slot() {
                    ValidateBlobOutcome::Discard(BlobDiscardReason::MaxAllowedUnregisteredBlobs)
                } else {
                    ValidateBlobOutcome::Accept(SequencerStatus::Unregistered)
                }
            }
        }
    }

    fn discard(
        discarded_blobs: &mut Vec<HexHash>,
        sender: &<S::Da as DaSpec>::Address,
        raw_blob_hash: [u8; 32],
        reason: &BlobDiscardReason,
    ) {
        let blob_hash = HexString(raw_blob_hash);
        info!(
            %blob_hash,
            sender = %sender,
            ?reason,
            "Discarding blob"
        );

        discarded_blobs.push(blob_hash);
    }

    /// Select blobs when transitioning from a preferred sequencer back to normal operation.
    /// This occurs when the preferred sequencer was slashed for malicious behavior. In recovery mode,
    /// the rollup processes two visible slots at a time until it catches up to the current slot, after
    /// which it performs standard based sequencing.
    fn select_blobs_in_recovery_mode(
        &mut self,
        current_blobs: RelevantBlobIters<&mut [<S::Da as DaSpec>::BlobTransaction]>,
        discarded_blobs: &mut Vec<HexHash>,
        state: &mut KernelStateAccessor<'_, S>,
    ) -> BlobSelectorOutput<ValidatedBlob<S, BatchWithId<S>>> {
        tracing::trace!("On recovery mode path");

        let delta = state
            .true_slot_number()
            .saturating_sub(state.visible_slot_number().get())
            .get();

        let mut blobs_with_total_size_limit = BlobsAccumulatorWithSizeLimit::<S>::new();

        // First, decide how many slots worth of stored blobs we need. It could be 0, 1, or 2.
        let (slots_needed_from_storage, current_orderred_blobs) = match delta {
            0 => panic!("Visible slot is the same as the true slot. Since true slot was already incremented this slot in .synchronize_chain() and visible slot is yet to be updated, this is a bug, please report it."),

            // If the visible slot is only trailing by one, we don't need to process any stored blobs.
            // We just need to process the new blobs from this slot. (We still return 1 slots_needed_from_storage, 
            // but since there is no stored blobs for the true_slot, this should be a no-op)
            1 => {
                let blobs = self.select_blobs_as_based_sequencer_inner(current_blobs, discarded_blobs, state);
                (1, Some(blobs))
            }
            // Otherwise, we need to process two slots from storage  - which means that we need to save the new blobs
            _ => {
                let new_batches: Vec<_> = self
                    .select_blobs_da_ordering(current_blobs,discarded_blobs, true, 2, state)
                    .into_iter()
                    .collect();
                self.store_batches(&new_batches, state);
                (2, None)
            }
        };

        if let Some(blobs) = current_orderred_blobs {
            for batch in blobs.selected_blobs.into_iter() {
                if let PushOrIgnore::IgnoredBlob(id, sender) =
                    blobs_with_total_size_limit.push_or_ignore(SequencerType::NonPreferred, batch)
                {
                    Self::discard(
                        discarded_blobs,
                        &sender,
                        id,
                        &BlobDiscardReason::OutOfCapacity,
                    );
                }
            }
        }

        // The virtual height increase is either 1 (if the delta is 0) or 2.
        let gas_price_for_new_block = self.get_new_gas_price(delta.min(2), state);
        self.retrieve_stored_blobs_and_add_to_selection(
            slots_needed_from_storage,
            &gas_price_for_new_block,
            &mut blobs_with_total_size_limit,
            discarded_blobs,
            state,
        );

        BlobSelectorOutput {
            selected_blobs: blobs_with_total_size_limit.inner(),
            visible_slot_number_increase: slots_needed_from_storage,
        }
    }

    fn separate_preferred_blobs<'a>(
        &self,
        blobs: &'a mut [<S::Da as DaSpec>::BlobTransaction],
        preferred_sender: &<S::Da as DaSpec>::Address,
    ) -> SeparatedBlobs<'a, S> {
        let mut preferred_blobs = Vec::new();
        let mut non_preferred_blobs = Vec::new();
        for blob in blobs {
            if &blob.sender() == preferred_sender {
                preferred_blobs.push(blob);
            } else {
                non_preferred_blobs.push(blob);
            }
        }
        SeparatedBlobs {
            preferred_blobs,
            non_preferred_blobs,
        }
    }

    /// Select the blobs to execute this slot based on the preferred sequencer and set the next visible_slot_number.
    ///
    /// During each `slot`, we process up to one `Batch` of transactions from the preferred sequencer, plus all of the proofs
    /// and batches that the preferred sequencer had seen by the time they submitted their batch. This is accomplished using
    /// a sequencer number (to prevent re-ordering of data submitted by the preferred sequencer) and a visible rollup height.
    /// In each of their batches, the sequencer gets to choose how far to advance the visible rollup height. For example, if
    /// the preferred sequencer had seen all the data up to slot 10 but hadn't posted a batch since slot 5, they would choose
    /// to increment the visible rollup height by 5 in their next on-chain message.
    ///
    /// During each `slot`, we process all proofs...
    /// - (If sent by the preferred sequencer) whose sequence number is less than that of the next preferred batch
    /// - (If sent by anyone else) which appeared on chain before or during the current *visible* rollup height
    /// For batches, we select
    /// - The next one sent by the preferred sequencer (if available)
    /// - Any batches which appeared on chain before or during the current *visible* slot number
    #[tracing::instrument(skip_all)]
    fn select_blobs_for_preferred_sequencer<'k, CF: InjectedControlFlow<S> + Clone>(
        &mut self,
        current_blobs: RelevantBlobIters<&mut [<S::Da as DaSpec>::BlobTransaction]>,
        discarded_blobs: &mut Vec<HexHash>,
        state: &mut KernelStateAccessor<'k, S>,
        preferred_sender: &<S::Da as DaSpec>::Address,
        preferred_sequencer: S::Address,
        cf: CF,
    ) -> BlobSelectorOutput<SelectedBlob<S, IterableBatchWithId<S, CF>>> {
        let mut sequence_tracker = self
            .upcoming_sequence_numbers
            .get(state)
            .unwrap_infallible()
            .unwrap_or_default();

        // 1. Extract all the new preferred blobs from the input

        let separated_proofs =
            self.separate_preferred_blobs(current_blobs.proof_blobs, preferred_sender);
        let separated_batches =
            self.separate_preferred_blobs(current_blobs.batch_blobs, preferred_sender);
        let well_formed_preferred_blobs = separated_proofs
            .preferred_blobs
            .into_iter()
            .map(BlobOrigin::Proof)
            .chain(
                separated_batches
                    .preferred_blobs
                    .into_iter()
                    .map(BlobOrigin::Batch),
            )
            .filter_map(|blob| match blob {
                BlobOrigin::Proof(proof_blob) => self
                    .deserialize_or_try_slash_sender::<PreferredProofData>(
                        proof_blob, None, true, state,
                    )
                    .map(|proof| PreferredBlobDataWithId {
                        inner: PreferredBlobData::Proof(proof),
                        id: proof_blob.hash().into(),
                    }),
                BlobOrigin::Batch(batch_blob) => self
                    .deserialize_or_try_slash_sender::<PreferredBatchData>(
                        batch_blob, None, true, state,
                    )
                    .map(|batch| PreferredBlobDataWithId {
                        inner: PreferredBlobData::Batch(batch),
                        id: batch_blob.hash().into(),
                    }),
            })
            .collect::<Vec<_>>();

        // 2. Select the preferred blobs to process
        let selected_preferred_blobs = self.pick_preferred_blobs_to_process(
            well_formed_preferred_blobs,
            discarded_blobs,
            &mut sequence_tracker,
            preferred_sender,
            state,
        );

        // 3. Select the virtual height increase
        let mut blobs_to_select = BlobsAccumulatorWithSizeLimit::<S>::new();
        let visible_height_increase = if let Some(last_selected_blob) =
            selected_preferred_blobs.last()
        {
            // If we have preferred blobs to process, the height increase is specified in the batch - which is the last item in the list of preferred blobs
            let visible_height_increase = {
                let requested_slots_to_advance = last_selected_blob.inner.visible_slot_number_increase()
                .expect("Decided to create a rollup block but the last item in the list of preferred blobs is not a batch. This is a bug.");

                let max_slots_to_advance = config_value!("MAX_VISIBLE_HEIGHT_INCREASE_PER_SLOT");
                let max_slots_to_advance = state
                    .true_slot_number()
                    .saturating_sub(state.visible_slot_number().get())
                    .get()
                    .try_into()
                    .unwrap_or(max_slots_to_advance)
                    .min(max_slots_to_advance);

                std::cmp::min(requested_slots_to_advance, max_slots_to_advance)
            };

            // 4. If there are preferred blobs, put them in the list of blobs to select
            self.add_preferred_blobs_to_selection(
                selected_preferred_blobs,
                &mut blobs_to_select,
                discarded_blobs,
                preferred_sender,
                &preferred_sequencer,
                visible_height_increase as u64,
                state,
            );
            visible_height_increase
        } else if state
            .visible_slot_number()
            .advance(config_deferred_slots_count())
            .as_true()
            <= state.true_slot_number()
        {
            // If the visible slot is lagging behind the current true slot number by the full DEFERRED_SLOTS_COUNT,
            // we need to force create a rollup block even though the preferred sequencer didn't request one
            1
        } else {
            0
        };

        // 5. Select the non-preferred blobs from storage
        let gas_price_for_new_block = self.get_new_gas_price(visible_height_increase as u64, state);
        // TODO: If we start dropping blobs on the *second* slot in this loop, the preferred sequencer is doing some sneaky censorship
        // The attack is not economically feasible (analysis available upon request).
        // by increasing the visible slot number too quickly, causing blobs to be dropped. We should consider slashing in this case.
        self.retrieve_stored_blobs_and_add_to_selection(
            visible_height_increase as u64,
            &gas_price_for_new_block,
            &mut blobs_to_select,
            discarded_blobs,
            state,
        );

        // 6. Select or defer the non-preferred blobs from the current slot
        let should_use_blobs_from_this_slot = state
            .visible_slot_number()
            .get()
            .saturating_add(visible_height_increase as u64)
            == state.true_slot_number().get();

        // If blobs from this slot are being used, we need to add them to the current blob limiter.
        // Otherwise, we need to create a new blob limiter for the new slot and save the resulting blobs into storage
        let all_non_preferred_blobs = separated_batches
            .non_preferred_blobs
            .into_iter()
            .map(BlobOrigin::Batch)
            .chain(
                separated_proofs
                    .non_preferred_blobs
                    .into_iter()
                    .map(BlobOrigin::Proof),
            );

        if should_use_blobs_from_this_slot {
            self.select_blobs_da_ordering_helper(
                all_non_preferred_blobs,
                &mut blobs_to_select,
                discarded_blobs,
                false,
                visible_height_increase as u64,
                state,
            );
        } else {
            let mut new_blob_deferral_limiter = BlobsAccumulatorWithSizeLimit::<S>::new();
            self.select_blobs_da_ordering_helper(
                all_non_preferred_blobs,
                &mut new_blob_deferral_limiter,
                discarded_blobs,
                true,
                visible_height_increase as u64,
                state,
            );
            self.store_batches(&new_blob_deferral_limiter.inner(), state);
        }

        BlobSelectorOutput {
            selected_blobs: blobs_to_select
                .inner()
                .into_iter()
                .map(|b| b.into_selected_blob(cf.clone()))
                .collect(),
            visible_slot_number_increase: visible_height_increase as u64,
        }
    }

    /// This helper function looks through the new preferred blobs and any existing ("deferred") blobs from the preferred sequencer to decide
    /// whether we should create a rollup block. It also saves any new preferred blobs that aren't going to be used immediately into storage.
    ///
    /// Recall that we create a rollup block if and only if we have a batch with the next sequencer number. If not for proofs, this function
    /// would be trivial. However, recall that proofs also have sequence numbers, but we can't create a block unless we have a *batch*.
    /// So, we need to look for the shortest run of consecutive sequence numbers that starts with `next_sequence_number` and ends with a batch.
    fn pick_preferred_blobs_to_process(
        &mut self,
        well_formed_preferred_blobs: Vec<PreferredBlobDataWithId>,
        discarded_blobs: &mut Vec<HexHash>,
        sequence_tracker: &mut SequencerNumberTracker,
        preferred_sender: &<S::Da as DaSpec>::Address,
        state: &mut KernelStateAccessor<'_, S>,
    ) -> Vec<PreferredBlobDataWithId> {
        let (next_run_of_blobs, new_blobs_to_defer, next_sequence_number) = self
            .find_next_run_of_blobs(
                well_formed_preferred_blobs,
                discarded_blobs,
                sequence_tracker,
                preferred_sender,
            );

        let blobs = self
            .get_blobs_to_process_from_run(
                next_run_of_blobs,
                new_blobs_to_defer,
                next_sequence_number,
                sequence_tracker,
                state,
            )
            .unwrap_or_default();

        assert!(
            blobs
                .windows(2)
                .all(|two_blobs| two_blobs[0].inner.sequence_number() + 1
                    == two_blobs[1].inner.sequence_number()),
            "Preferred blobs to process must have consecutive sequence numbers. This is a bug, please report it"
        );
        assert!(
            blobs.iter().filter(|blob| blob.inner.is_batch()).count() <= 1,
            "Can't have more than one preferred batch per run. This is a bug, please report it"
        );

        blobs
    }

    /// Returns the next run of blobs to process, an iterator over the blobs that weren't even considered, and the next sequence number
    ///
    /// Note that the next run of blobs may not contain a batch! If it doesn't, a rollup block will not be created.
    fn find_next_run_of_blobs<'a>(
        &self,
        mut well_formed_preferred_blobs: Vec<PreferredBlobDataWithId>,
        discarded_blobs: &mut Vec<HexHash>,
        sequence_tracker: &SequencerNumberTracker,
        preferred_sender: &'a <S::Da as DaSpec>::Address,
    ) -> (
        Vec<BlobArrival>,
        impl Iterator<Item = PreferredBlobDataWithId> + 'a,
        u64,
    ) {
        let mut next_sequence_number = sequence_tracker.next_sequence_number;
        let mut next_run_of_blobs = Vec::new();
        let mut stored_sequencer_numbers =
            sequence_tracker.saved_sequencer_numbers.iter().peekable();
        // Sort the preferred blobs by sequence number, and make an iterator over all the ones with new sequence numbers
        well_formed_preferred_blobs.sort_by_key(|blob| blob.inner.sequence_number());

        well_formed_preferred_blobs.retain(|blob| {
            if blob.inner.sequence_number() >= next_sequence_number {
                true
            } else {
                Self::discard(
                    discarded_blobs,
                    preferred_sender,
                    blob.id,
                    &BlobDiscardReason::SequenceNumberTooLow,
                );
                false
            }
        });

        let mut new_blobs = well_formed_preferred_blobs.into_iter().peekable();

        let next_sequence_number = loop {
            // First, see if the next *new* blob has the next sequence number
            if let Some(blob) = new_blobs.peek() {
                // If so, add it to the list of blobs to process and advance the sequence number
                if blob.inner.sequence_number() == next_sequence_number {
                    let blob = new_blobs.next().unwrap();
                    let is_batch = blob.inner.is_batch();
                    next_sequence_number += 1;
                    next_run_of_blobs.push(BlobArrival::New(blob));
                    // If we've found a batch, we're done
                    if is_batch {
                        break next_sequence_number;
                    } else {
                        continue;
                    }
                }
            }
            // If we make it to this point, the next new blob didn't have our sequence number. Check if the blob we need is already in storage
            if let Some((sequence_number, _)) = stored_sequencer_numbers.peek() {
                // If the next blob in storage has the next sequence number, add it to the list of blobs to process and advance the sequence number
                if **sequence_number == next_sequence_number {
                    let (sequence_number, blob_type) = stored_sequencer_numbers.next().unwrap();
                    next_run_of_blobs.push(BlobArrival::Stored(*sequence_number, *blob_type));
                    next_sequence_number += 1;
                    if blob_type.is_batch() {
                        break next_sequence_number;
                    }
                    continue;
                }
            }
            // If we reach this point, neither the list of new blobs nor the list of stored blobs has the next sequence number. We're stuck. break and returne the old sequence number
            break sequence_tracker.next_sequence_number;
        };

        (next_run_of_blobs, new_blobs, next_sequence_number)
    }

    fn retrieve_stored_blobs_and_add_to_selection(
        &mut self,
        slots_needed_from_storage: u64,
        gas_price_for_new_block: &<S::Gas as Gas>::Price,
        blobs_with_total_size_limit: &mut BlobsAccumulatorWithSizeLimit<S>,
        discarded_blobs: &mut Vec<HexHash>,
        state: &mut KernelStateAccessor<'_, S>,
    ) {
        for slot in 1..=slots_needed_from_storage {
            let slot_to_check = state.visible_slot_number().saturating_add(slot);
            let batches_from_next_slot = self.take_blobs_for_slot(slot_to_check, state);

            for mut batch in batches_from_next_slot {
                // For each batch we retrieve from storage, we check if it has enough reserved gas to cover the pre-execution checks.
                // If so, we select it for execution. If not, we drop the blob and refund the reserved gas to the sequencer.
                let balance_store = &batch.balance_store;
                match balance_store {
                    Escrow::DerivedHolder(reserved_balance) => {
                        if let Ok(retrieved_token_amount) = self.move_funds_from_escrow_to_bank(&batch, reserved_balance, gas_price_for_new_block, state) {
                            let _ = std::mem::replace(&mut batch.balance_store, Escrow::Direct(retrieved_token_amount));
                        } else {
                            Self::discard(
                                discarded_blobs,
                                &batch.sender,
                                batch.blob.id(),
                                &BlobDiscardReason::InsufficientReservedGas,
                            );
                            // If we can't retrieve enough funds from escrow, we drop the blob and move on to the next one.
                            continue;
                        }
                    }
                    Escrow::Direct(_) => unreachable!("Deferred blobs must store their gas in a derived account until it's ready to be used."),
                    Escrow::None => {}

                }
                if let PushOrIgnore::IgnoredBlob(id, sender) =
                    blobs_with_total_size_limit.push_or_ignore(SequencerType::NonPreferred, batch)
                {
                    Self::discard(
                        discarded_blobs,
                        &sender,
                        id,
                        &BlobDiscardReason::OutOfCapacity,
                    );
                }
            }
        }
    }

    fn move_funds_from_escrow_to_bank(
        &mut self,
        batch: &ValidatedBlob<S, BatchWithId<S>>,
        escrow: &DerivedHolder,
        gas_price_for_new_block: &<S::Gas as Gas>::Price,
        state: &mut KernelStateAccessor<'_, S>,
    ) -> Result<Amount, anyhow::Error> {
        let refund_recipient = match &batch.blob {
            BlobDataWithId::Batch(b) => &b.sequencer_address,
            BlobDataWithId::EmergencyRegistration { .. } => panic!("Emergency registrations don't reserve gas because the sender is unknown. This is a bug!"),
            BlobDataWithId::Proof { sequencer_address, .. } => sequencer_address,
        };
        if let Some(gas_needed_for_pre_exec_checks) = <S as GasSpec>::max_tx_check_costs()
            .checked_scalar_product(Self::num_pre_exec_checks_needed(&batch.blob) as u64)
            .and_then(|gas_needed| gas_needed.checked_value(gas_price_for_new_block))
        {
            let retrieval_result = self.sequencer_registry.retrieve_funds_from_escrow(
                escrow,
                refund_recipient,
                gas_needed_for_pre_exec_checks,
                state,
            );
            if retrieval_result.is_ok() {
                return Ok(gas_needed_for_pre_exec_checks);
            }
        }

        tracing::warn!("Unable to pay pre-execution costs out of reserved gas balance for batch {}. Dropping it. {} will have their remaining reserved balance refunded.", hex::encode(batch.blob.id()), refund_recipient);
        self.sequencer_registry
            .refund_all_reserved_gas(escrow, refund_recipient, state);
        anyhow::bail!("Unable to reserve all needed gas.");
    }

    #[allow(clippy::too_many_arguments)]
    fn add_preferred_blobs_to_selection(
        &mut self,
        selected_preferred_blobs: Vec<PreferredBlobDataWithId>,
        blobs_to_select: &mut BlobsAccumulatorWithSizeLimit<S>,
        discarded_blobs: &mut Vec<HexHash>,
        preferred_sender: &<S::Da as DaSpec>::Address,
        preferred_sequencer: &S::Address,

        visible_height_increase: u64,
        state: &mut KernelStateAccessor<'_, S>,
    ) {
        for blob in selected_preferred_blobs {
            let blob_id = blob.id;
            let data = match blob.inner {
                PreferredBlobData::Batch(batch) => {
                    BlobData::Batch((batch.data, preferred_sequencer.clone()))
                }
                PreferredBlobData::Proof(proof) => {
                    BlobData::Proof((proof.data, preferred_sequencer.clone()))
                }
            };
            let blob_with_id = data.with_id(blob_id);

            let available_balance = self
                .sequencer_registry
                .get_sender_balance(preferred_sender, state)
                .unwrap_or(Amount::ZERO);

            let Some(validated_blob) = self.validate_preferred_blob(
                blob_with_id,
                preferred_sender.clone(),
                available_balance,
                blobs_to_select,
                visible_height_increase,
                state,
            ) else {
                tracing::error!(blob_id = hex::encode(blob_id), "Preferred sequencer did not have enough balance to submit this blob. Dropping preferred blob. Some soft confirmations may be invalidated");
                Self::discard(
                    discarded_blobs,
                    preferred_sender,
                    blob_id,
                    &BlobDiscardReason::SenderInsufficientStake,
                );

                continue;
            };
            if let PushOrIgnore::IgnoredBlob(id, sender) =
                blobs_to_select.push_or_ignore(SequencerType::Preferred, validated_blob)
            {
                Self::discard(
                    discarded_blobs,
                    &sender,
                    id,
                    &BlobDiscardReason::OutOfCapacity,
                );
                tracing::error!(blob_id = hex::encode(blob_id), "Preferred blob size limit exceeded. Dropping preferred blob. Some soft confirmations may be invalidated");
            }
        }
    }

    fn try_validate_proof_and_reserve_funds(
        &mut self,
        idx: u32,
        blob: &mut <S::Da as DaSpec>::BlobTransaction,
        account_for_deferral: bool,
        visible_height_increase: u64,
        state: &mut KernelStateAccessor<'_, S>,
    ) -> Option<ValidatedBlob<S, BatchWithId<S>>> {
        // Proofs must come from a registered sequencer
        let Ok(sequencer) = self
            .sequencer_registry
            .is_sender_allowed(&blob.sender(), state)
        else {
            return None;
        };
        let gas_price_for_new_block: <<S as Spec>::Gas as Gas>::Price =
            self.get_new_gas_price(visible_height_increase, state);

        let proof = self.deserialize_or_try_slash_sender::<Vec<u8>>(
            blob,
            Some((&sequencer, &gas_price_for_new_block)),
            true,
            state,
        )?;

        let available_balance = self
            .sequencer_registry
            .get_sender_balance(&blob.sender(), state)
            .unwrap_or(Amount::ZERO);

        self.validate_blob(
            idx,
            BlobData::Proof((proof, sequencer.address)).with_id(blob.hash().into()),
            blob.sender(),
            available_balance,
            &gas_price_for_new_block,
            account_for_deferral,
            state,
        )
    }

    fn try_validate_batch_and_reserve_funds_if_needed(
        &mut self,
        idx: u32,
        blob: &mut <S::Da as DaSpec>::BlobTransaction,
        sequencer: AllowedSequencer<S>,
        gas_price_for_new_block: &<S::Gas as Gas>::Price,
        account_for_deferral: bool,
        state: &mut KernelStateAccessor<'_, S>,
    ) -> Option<ValidatedBlob<S, BatchWithId<S>>> {
        // This is checked elsewhere, but we check it again here before doing anything that might impact the sender's balance.
        // Defense in depth.
        let batch = self.deserialize_or_try_slash_sender::<Vec<FullyBakedTx>>(
            blob,
            Some((&sequencer, gas_price_for_new_block)),
            true,
            state,
        )?;

        let available_balance = self
            .sequencer_registry
            .get_sender_balance(&blob.sender(), state)
            .unwrap_or(Amount::ZERO);

        self.validate_blob(
            idx,
            BlobData::Batch((Arc::new(batch), sequencer.address)).with_id(blob.hash().into()),
            blob.sender(),
            available_balance,
            gas_price_for_new_block,
            account_for_deferral,
            state,
        )
    }

    /// Takes the next run of blobs and extracts the list  of blobs that should be processed this slot, if any. Saves
    /// any new preferred blobs that aren't going to be used immediately into storage.
    fn get_blobs_to_process_from_run(
        &mut self,
        next_run_of_blobs: Vec<BlobArrival>,
        new_blobs_not_processed: impl Iterator<Item = PreferredBlobDataWithId>,
        next_sequence_number: u64,
        old_sequence_tracker: &mut SequencerNumberTracker,
        state: &mut KernelStateAccessor<'_, S>,
    ) -> Option<Vec<PreferredBlobDataWithId>> {
        let mut next_sequence_tracker = SequencerNumberTracker {
            next_sequence_number,
            saved_sequencer_numbers: old_sequence_tracker
                .saved_sequencer_numbers
                .split_off(&next_sequence_number),
        };
        // Any additional new blobs that aren't being used need to be put into storage.
        for blob in new_blobs_not_processed {
            next_sequence_tracker
                .saved_sequencer_numbers
                .insert(blob.inner.sequence_number(), blob.inner.blob_type());
            self.deferred_preferred_sequencer_blobs
                .set(&blob.inner.sequence_number(), &blob, state)
                .unwrap_infallible();
        }

        let create_rollup_block = next_run_of_blobs
            .last()
            .map(|blob| blob.blob_type().is_batch())
            .unwrap_or(false);
        // If we're not creating a rollup block, we need to store all of the new preferred blobs in storage.
        if !create_rollup_block {
            for blob in next_run_of_blobs {
                if let BlobArrival::New(blob) = blob {
                    next_sequence_tracker
                        .saved_sequencer_numbers
                        .insert(blob.inner.sequence_number(), blob.inner.blob_type());
                    self.deferred_preferred_sequencer_blobs
                        .set(&blob.inner.sequence_number(), &blob, state)
                        .unwrap_infallible();
                }
            }
            self.upcoming_sequence_numbers
                .set(&next_sequence_tracker, state)
                .unwrap_infallible();
            return None;
        }

        let mut output = Vec::with_capacity(next_run_of_blobs.len());
        self.upcoming_sequence_numbers
            .set(&next_sequence_tracker, state)
            .unwrap_infallible();
        for blob in next_run_of_blobs {
            match blob {
                BlobArrival::New(blob) => {
                    output.push(blob);
                }
                BlobArrival::Stored(sequence_number, _) => {
                    let blob_content = self
                        .deferred_preferred_sequencer_blobs
                        .remove(&sequence_number, state)
                        .unwrap_infallible()
                        .expect("Blob was present in index but not in storage. This is a bug.");
                    output.push(blob_content);
                }
            }
        }
        Some(output)
    }

    /// Deserialize a blob into a `Batch` or slash the sender if it's malformed.
    /// The sequencer might not exist if we're processing a blob submitted by an unregistered
    /// sequencer - in the case of direct sequencer registration via DA.
    fn deserialize_or_try_slash_sender<B: BorshDeserialize>(
        &mut self,
        blob: &mut <S::Da as DaSpec>::BlobTransaction,
        charge_for_deserialization: Option<(&AllowedSequencer<S>, &<S::Gas as Gas>::Price)>,
        slash_on_failure: bool,
        state: &mut KernelStateAccessor<'_, S>,
    ) -> Option<B> {
        if let Some((registered_sender, gas_price_for_new_block)) = charge_for_deserialization {
            let funds_for_deserialization =
                <S as GasSpec>::gas_to_charge_per_byte_borsh_deserialization()
                    .checked_scalar_product(blob.total_len() as u64)?
                    .checked_value(gas_price_for_new_block)?;
            if registered_sender.balance < funds_for_deserialization {
                return None;
            }
            // Burn the cost of deserialization from the sender's balance. For now, we just send it to the sequencer registry where it'll remain inaccessible.
            self.sequencer_registry.remove_part_of_the_stake(
                &blob.sender(),
                self.sequencer_registry.id().clone().to_payable(),
                funds_for_deserialization,
                state,
            ).expect("Failed to remove funds for deserialization even though the sender has enough balance. This should never happen.");
        }
        match B::try_from_slice(data_for_deserialization(blob)) {
            Ok(batch) => Some(batch),
            // if the blob is malformed, slash the sequencer
            Err(e) => {
                assert_eq!(blob.verified_data().len(), blob.total_len(), "Batch deserialization failed and some data was not provided. The prover might be malicious");
                let leading_bytes =
                    &blob.verified_data()[..std::cmp::min(100, blob.verified_data().len())];
                trace!(
                    deserializing_as = std::any::type_name::<B>(),
                    leading_bytes = %hex::encode(leading_bytes),
                    "Deserializing blob"
                );
                debug!(
                    blob_hash = %blob.hash(),
                    slashed_sender = %blob.sender(),
                    error = ?e,
                    "Unable to deserialize blob. slashing sender if they are registered"
                );

                if slash_on_failure {
                    self.sequencer_registry
                        .slash_sequencer(&blob.sender(), state);
                } else {
                    info!(sender = %blob.sender(), "Unable to slash sequencer, they were not registered");
                }

                None
            }
        }
    }
}

// The public API of the BlobStorage module.
impl<S: Spec> BlobStorage<S> {
    #[allow(clippy::type_complexity)]
    /// This implementation returns three categories of blobs:
    /// 1. Any blobs sent by the preferred sequencer ("prority blobs")
    /// 2. Any non-priority blobs which were sent `DEFERRED_SLOTS_COUNT` slots ago ("expiring deferred blobs")
    /// 3. Some additional deferred blobs needed to fill the total requested by the sequencer, if applicable. ("bonus blobs")
    pub fn get_blobs_for_this_slot<CF: InjectedControlFlow<S> + Clone>(
        &mut self,
        current_blobs: RelevantBlobIters<&mut [<S::Da as DaSpec>::BlobTransaction]>,
        state: &mut KernelStateAccessor<'_, S>,
        cf: CF,
    ) -> anyhow::Result<(
        BlobSelectorOutput<SelectedBlob<S, IterableBatchWithId<S, CF>>>,
        Vec<HexHash>,
    )> {
        let mut discarded_blobs = Vec::default();

        // If `DEFERRED_SLOTS_COUNT` is 0, we treat the rollup as having no preferred sequencer.
        // In this case, we just process blobs in the order that they appeared on the DA layer
        if config_deferred_slots_count() == 0 {
            let selection = self.select_blobs_as_based_sequencer_inner(
                current_blobs,
                &mut discarded_blobs,
                state,
            );

            return Ok((
                BlobSelectorOutput {
                    selected_blobs: selection
                        .selected_blobs
                        .into_iter()
                        .map(|b| b.into_selected_blob(cf.clone()))
                        .collect(),
                    visible_slot_number_increase: selection.visible_slot_number_increase,
                },
                discarded_blobs,
            ));
        }

        // If there's a preferred sequencer, sequence accordingly.
        if let Some((pref_da, pref_seq)) = self.get_preferred_sequencer(state) {
            return Ok((
                self.select_blobs_for_preferred_sequencer(
                    current_blobs,
                    &mut discarded_blobs,
                    state,
                    &pref_da,
                    pref_seq,
                    cf,
                ),
                discarded_blobs,
            ));
        }

        // Otherwise, we're configured for a preferred sequencer but one doesn't exist. This usually means that the preferred sequencer was slashed.
        // Entery recovery mode.
        let selection =
            self.select_blobs_in_recovery_mode(current_blobs, &mut discarded_blobs, state);

        Ok((
            BlobSelectorOutput {
                selected_blobs: selection
                    .selected_blobs
                    .into_iter()
                    .map(|b| b.into_selected_blob(cf.clone()))
                    .collect(),
                visible_slot_number_increase: selection.visible_slot_number_increase,
            },
            discarded_blobs,
        ))
    }

    /// Escrow funds for the preferred sequencer.
    pub fn escrow_funds_for_preferred_sequencer(
        &mut self,
        amount: Amount,
        state: &mut KernelStateAccessor<'_, S>,
    ) -> anyhow::Result<()> {
        let preferred_sequencer = self
            .sequencer_registry
            .preferred_sequencer(state)
            .expect("Preferred sequencer must be set in order to escrow funds!");

        self.sequencer_registry.remove_part_of_the_stake(
            &preferred_sequencer,
            self.bank.id().to_payable(),
            amount,
            state,
        )
    }

    #[allow(clippy::type_complexity)]
    /// Select the blobs to execute this slot using "based sequencing". In this mode,
    /// blobs are processed in the order that they appear on the DA layer.
    pub fn select_blobs_as_based_sequencer<CF: InjectedControlFlow<S> + Clone>(
        &mut self,
        current_blobs: RelevantBlobIters<&mut [<<S as Spec>::Da as DaSpec>::BlobTransaction]>,
        state: &mut KernelStateAccessor<'_, S>,
        cf: CF,
    ) -> (
        BlobSelectorOutput<SelectedBlob<S, IterableBatchWithId<S, CF>>>,
        Vec<HexHash>,
    ) {
        let mut discarded_blobs = Vec::default();
        let output =
            self.select_blobs_as_based_sequencer_inner(current_blobs, &mut discarded_blobs, state);
        (
            BlobSelectorOutput {
                selected_blobs: output
                    .selected_blobs
                    .into_iter()
                    .map(|b| b.into_selected_blob(cf.clone()))
                    .collect(),
                visible_slot_number_increase: output.visible_slot_number_increase,
            },
            discarded_blobs,
        )
    }

    /// Extracts all delayed non-preferred blobs that belong to the given slots.
    pub fn get_non_preferred_blobs(
        &mut self,
        slot_range: impl Iterator<Item = SlotNumber>,
        state: &mut KernelStateAccessor<'_, S>,
    ) -> Vec<ValidatedBlob<S, BatchWithId<S>>> {
        let discarded_blobs = &mut Vec::default();
        let mut blobs_with_total_size_limit = BlobsAccumulatorWithSizeLimit::<S>::new();

        // Load all the necessary batches from storage.
        for slot_to_check in slot_range {
            let batches_from_next_slot = self.take_blobs_for_slot(slot_to_check, state);
            tracing::trace!(
                "Found {} additional blobs in slot {} ",
                batches_from_next_slot.len(),
                slot_to_check
            );
            for batch in batches_from_next_slot {
                // Only push the blobs that are within the total size limit.
                if let PushOrIgnore::IgnoredBlob(id, sender) =
                    blobs_with_total_size_limit.push_or_ignore(SequencerType::NonPreferred, batch)
                {
                    Self::discard(
                        discarded_blobs,
                        &sender,
                        id,
                        &BlobDiscardReason::OutOfCapacity,
                    );
                }
            }
        }

        blobs_with_total_size_limit.inner()
    }
}

#[cfg(feature = "native")]
fn data_for_deserialization(blob: &mut impl BlobReaderTrait) -> &[u8] {
    blob.full_data()
}

#[cfg(not(feature = "native"))]
fn data_for_deserialization(blob: &mut impl BlobReaderTrait) -> &[u8] {
    blob.verified_data()
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU8;

    use sov_mock_da::MOCK_SEQUENCER_DA_ADDRESS;
    use sov_test_utils::TestSpec;

    use super::{
        BlobStorage, BlobType, PreferredBatchData, PreferredBlobData, PreferredBlobDataWithId,
        PreferredProofData, SequencerNumberTracker,
    };

    #[test]
    fn test_find_next_run() {
        use BlobType::*;

        let blobs = [(Batch, 1), (Proof, 0)];
        let expected_output = ExpectedOutput {
            next_run_of_blobs: vec![0, 1],
            blobs_to_defer: vec![],
            next_sequence_number: 2,
        };
        do_blob_test(
            &blobs,
            &mut SequencerNumberTracker::default(),
            &expected_output,
        );
    }

    #[test]
    fn test_find_next_run_start_with_saved_proof() {
        use BlobType::*;

        let mut tracker = SequencerNumberTracker {
            next_sequence_number: 5,
            saved_sequencer_numbers: [(5, Proof), (8, Batch)].into_iter().collect(),
        };
        let blobs = [(Proof, 6), (Batch, 7), (Batch, 9)];
        let expected_output = ExpectedOutput {
            next_run_of_blobs: vec![5, 6, 7],
            blobs_to_defer: vec![9],
            next_sequence_number: 8,
        };
        do_blob_test(&blobs, &mut tracker, &expected_output);
    }

    #[test]
    fn test_find_next_run_start_with_saved_batch() {
        use BlobType::*;
        let mut tracker = SequencerNumberTracker {
            next_sequence_number: 8,
            saved_sequencer_numbers: [(8, Batch), (9, Batch)].into_iter().collect(),
        };
        let blobs = [];
        let expected_output = ExpectedOutput {
            next_run_of_blobs: vec![8],
            blobs_to_defer: vec![],
            next_sequence_number: 9,
        };
        do_blob_test(&blobs, &mut tracker, &expected_output);
    }

    #[test]
    fn test_find_next_run_no_batch_saved_proof() {
        use BlobType::*;
        let mut tracker = SequencerNumberTracker {
            next_sequence_number: 3,
            saved_sequencer_numbers: [(3, Proof)].into_iter().collect(),
        };
        let blobs = [];
        let expected_output = ExpectedOutput {
            next_run_of_blobs: vec![3],
            blobs_to_defer: vec![],
            next_sequence_number: 3, // Since this run doesn't end in a batch, we won't actually execute it - so the sequence number is unchanged
        };
        do_blob_test(&blobs, &mut tracker, &expected_output);
    }

    #[test]
    fn test_find_next_run_no_batch_fresh_proof() {
        use BlobType::*;
        let mut tracker = SequencerNumberTracker {
            next_sequence_number: 3,
            saved_sequencer_numbers: [].into_iter().collect(),
        };
        let blobs = [(Proof, 3)];
        let expected_output = ExpectedOutput {
            next_run_of_blobs: vec![3],
            blobs_to_defer: vec![],
            next_sequence_number: 3, // Since this run doesn't end in a batch, we won't actually execute it - so the sequence number is unchanged
        };
        do_blob_test(&blobs, &mut tracker, &expected_output);
    }

    struct ExpectedOutput {
        next_run_of_blobs: Vec<u64>,
        blobs_to_defer: Vec<u64>,
        next_sequence_number: u64,
    }

    fn create_blob(blob_type: BlobType, sequence_number: u64, idx: u8) -> PreferredBlobDataWithId {
        let inner = match blob_type {
            BlobType::Batch => PreferredBlobData::Batch(PreferredBatchData {
                sequence_number,
                data: vec![].into(),
                visible_slots_to_advance: NonZeroU8::new(1).unwrap(),
            }),
            BlobType::Proof => PreferredBlobData::Proof(PreferredProofData {
                sequence_number,
                data: vec![],
            }),
        };
        PreferredBlobDataWithId {
            inner,
            id: [idx; 32],
        }
    }

    fn do_blob_test(
        slot: &[(BlobType, u64)],
        sequence_tracker: &mut SequencerNumberTracker,
        expected_output: &ExpectedOutput,
    ) {
        let discarded_blobs = &mut Vec::default();
        let sequencer_address = MOCK_SEQUENCER_DA_ADDRESS.into();
        let blob_storage = BlobStorage::<TestSpec>::default();

        // for slot in slots {
        let mut slot_blobs = Vec::new();
        for (idx, blob) in slot.iter().enumerate() {
            let blob = create_blob(blob.0, blob.1, idx.try_into().unwrap());
            slot_blobs.push(blob);
        }
        let (blobs_to_process, blobs_to_defer, next_sequence_number) = blob_storage
            .find_next_run_of_blobs(
                slot_blobs,
                discarded_blobs,
                sequence_tracker,
                &sequencer_address,
            );
        assert_eq!(
            blobs_to_process
                .into_iter()
                .map(|blob| blob.sequence_number())
                .collect::<Vec<_>>(),
            expected_output.next_run_of_blobs
        );
        assert_eq!(
            blobs_to_defer
                .into_iter()
                .map(|blob| blob.inner.sequence_number())
                .collect::<Vec<_>>(),
            expected_output.blobs_to_defer
        );
        assert_eq!(next_sequence_number, expected_output.next_sequence_number);
    }
}
