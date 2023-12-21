use sov_chain_state::TransitionHeight;
use sov_modules_api::prelude::*;
use sov_modules_api::runtime::capabilities::{BlobRefOrOwned, BlobSelector};
use sov_modules_api::{BlobReaderTrait, Context, DaSpec, KernelWorkingSet, WorkingSet};
use tracing::info;

use crate::{BlobStorage, DEFERRED_SLOTS_COUNT};

impl<C: Context, Da: DaSpec> BlobStorage<C, Da> {
    fn filter_by_allowed_sender(
        &self,
        b: &Da::BlobTransaction,
        working_set: &mut WorkingSet<C>,
    ) -> bool {
        {
            let is_allowed = self
                .sequencer_registry
                .is_sender_allowed(&b.sender(), working_set);
            // This is the best effort approach for making sure,
            // that blobs do not disappear silently
            // TODO: Add issue for that
            if !is_allowed {
                info!(
                    "Blob hash=0x{} from sender {} is going to be discarded",
                    hex::encode(b.hash()),
                    b.sender()
                );
            }
            is_allowed
        }
    }
}

impl<C: Context, Da: DaSpec> BlobSelector<Da> for BlobStorage<C, Da> {
    type Context = C;

    // This implementation returns three categories of blobs:
    // 1. Any blobs sent by the preferred sequencer ("prority blobs")
    // 2. Any non-priority blobs which were sent `DEFERRED_SLOTS_COUNT` slots ago ("expiring deferred blobs")
    // 3. Some additional deferred blobs needed to fill the total requested by the sequencer, if applicable. ("bonus blobs")
    fn get_blobs_for_this_slot<'a, 'k, I>(
        &self,
        current_blobs: I,
        working_set: &mut KernelWorkingSet<'k, C>,
    ) -> anyhow::Result<Vec<BlobRefOrOwned<'a, Da::BlobTransaction>>>
    where
        I: IntoIterator<Item = &'a mut Da::BlobTransaction>,
    {
        // If `DEFERRED_SLOTS_COUNT` is 0, we don't never to do any deferred blob processing and this
        // function just sorts and filters the current blobs before returning
        if DEFERRED_SLOTS_COUNT == 0 {
            let mut blobs = current_blobs
                .into_iter()
                .filter(|b| self.filter_by_allowed_sender(b, working_set.inner))
                .map(Into::into)
                .collect::<Vec<_>>();
            if let Some(sequencer) = self.get_preferred_sequencer(working_set.inner) {
                blobs.sort_by_key(|b: &BlobRefOrOwned<Da::BlobTransaction>| {
                    b.as_ref().sender() != sequencer
                });
            }
            return Ok(blobs.into_iter().map(Into::into).collect());
        }

        // Calculate any expiring deferred blobs first, since these have to be processed no matter what (Case 2 above).
        // Note that we have to handle this case even if there is no preferred sequencer, since that sequencer might have
        // exited while there were deferred blobs waiting to be processed
        let current_slot: TransitionHeight = self.get_true_slot_height(working_set);
        let slot_for_expiring_blobs =
            current_slot.saturating_sub(self.get_deferred_slots_count(working_set.inner));
        let expiring_deferred_blobs: Vec<Da::BlobTransaction> =
            self.take_blobs_for_slot_height(slot_for_expiring_blobs, working_set.inner);

        // If there is no preferred sequencer, that's all we need to do
        let preferred_sequencer =
            if let Some(sequencer) = self.get_preferred_sequencer(working_set.inner) {
                sequencer
            } else {
                // TODO: https://github.com/Sovereign-Labs/sovereign-sdk/issues/654
                // Prevent double number of blobs being executed
                return Ok(expiring_deferred_blobs
                    .into_iter()
                    .map(Into::into)
                    .chain(current_blobs.into_iter().map(Into::into))
                    .collect());
            };

        // If we reach this point, there is a preferred sequencer, so we need to handle cases 1 and 3.

        // First, compute any "bonus blobs" requested
        // to be processed early.
        let num_bonus_blobs_requested = self
            .deferred_blobs_requested_for_execution_next_slot
            .get(working_set.inner)
            .unwrap_or_default();
        self.deferred_blobs_requested_for_execution_next_slot
            .set(&0, working_set.inner);

        let mut remaining_blobs_requested =
            (num_bonus_blobs_requested as usize).saturating_sub(expiring_deferred_blobs.len());
        let mut bonus_blobs: Vec<BlobRefOrOwned<Da::BlobTransaction>> =
            Vec::with_capacity(remaining_blobs_requested);
        let mut next_slot_to_check = slot_for_expiring_blobs + 1;
        // We only need to check slots up to the current slot, since deferred blobs from the current
        // slot haven't been stored yet. We'll handle those later.
        while remaining_blobs_requested > 0 && next_slot_to_check < current_slot {
            let mut blobs_from_next_slot =
                self.take_blobs_for_slot_height(next_slot_to_check, working_set.inner);

            // If the set of deferred blobs from the next slot in line contains more than the remainder needed to fill the request,
            //  we split that group and save the unused portion back into state
            if blobs_from_next_slot.len() > remaining_blobs_requested {
                let blobs_to_save: Vec<<Da as DaSpec>::BlobTransaction> =
                    blobs_from_next_slot.split_off(remaining_blobs_requested);
                bonus_blobs.extend(blobs_from_next_slot.into_iter().map(Into::into));
                self.store_blobs(
                    next_slot_to_check,
                    &blobs_to_save.iter().collect::<Vec<_>>(),
                    working_set.inner,
                )?;
                remaining_blobs_requested = 0;
                break;
            } else {
                remaining_blobs_requested -= blobs_from_next_slot.len();
                bonus_blobs.extend(blobs_from_next_slot.into_iter().map(Into::into));
            }
            next_slot_to_check += 1;
        }

        // Finally handle any new blobs which appeared on the DA layer in this slot
        let mut priority_blobs = Vec::new();
        let mut to_defer: Vec<&mut Da::BlobTransaction> = Vec::new();
        for blob in current_blobs {
            // Blobs from the preferred sequencer get priority
            if blob.sender() == preferred_sequencer {
                priority_blobs.push(blob);
            } else {
                // Other blobs get deferred unless the sequencer has requested otherwise
                if remaining_blobs_requested > 0 {
                    remaining_blobs_requested -= 1;
                    bonus_blobs.push(blob.into())
                } else {
                    to_defer.push(blob);
                }
            }
        }

        // Save any blobs that need deferring
        if !to_defer.is_empty() {
            // TODO: https://github.com/Sovereign-Labs/sovereign-sdk/issues/655
            // Gas metering suppose to prevent saving blobs from not allowed senders if they exit mid-slot
            let to_defer: Vec<&Da::BlobTransaction> = to_defer
                .iter()
                .filter(|b| self.filter_by_allowed_sender(b, working_set.inner))
                .map(|b| &**b)
                .collect();
            self.store_blobs(current_slot, &to_defer, working_set.inner)?
        }

        Ok(priority_blobs
            .into_iter()
            .map(Into::into)
            .chain(expiring_deferred_blobs.into_iter().map(Into::into))
            .chain(bonus_blobs)
            .collect())
    }
}
