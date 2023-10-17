use sov_chain_state::TransitionHeight;
use sov_modules_api::capabilities::{BlobRefOrOwned, BlobSelector};
use sov_modules_api::{BlobReaderTrait, Context, DaSpec, WorkingSet};
use tracing::info;

use crate::BlobStorage;

impl<C: Context, Da: DaSpec> BlobSelector<Da> for BlobStorage<C, Da> {
    type Context = C;

    fn get_blobs_for_this_slot<'a, I>(
        &self,
        current_blobs: I,
        working_set: &mut WorkingSet<C>,
    ) -> anyhow::Result<Vec<BlobRefOrOwned<'a, Da::BlobTransaction>>>
    where
        I: IntoIterator<Item = &'a mut Da::BlobTransaction>,
    {
        let preferred_sequencer = self.get_preferred_sequencer(working_set);

        let current_slot: TransitionHeight = self.get_current_slot_height(working_set);
        let mut slot_for_next_blobs =
            current_slot.saturating_sub(self.get_deferred_slots_count(working_set));
        let mut past_deferred: Vec<Da::BlobTransaction> =
            if current_slot < self.get_deferred_slots_count(working_set) {
                Default::default()
            } else {
                self.take_blobs_for_slot_height(slot_for_next_blobs, working_set)
            };
        let preferred_sequencer = self.get_preferred_sequencer(working_set);

        let preferred_sequencer = if let Some(sequencer) = preferred_sequencer {
            sequencer
        } else {
            // TODO: https://github.com/Sovereign-Labs/sovereign-sdk/issues/654
            // Prevent double number of blobs being executed
            return Ok(past_deferred
                .into_iter()
                .map(Into::into)
                .chain(current_blobs.into_iter().map(Into::into))
                .collect());
        };

        let mut priority_blobs = Vec::new();
        let mut to_defer: Vec<&mut Da::BlobTransaction> = Vec::new();

        for blob in current_blobs {
            if blob.sender() == preferred_sequencer {
                priority_blobs.push(blob);
            } else {
                to_defer.push(blob);
            }
        }

        if !to_defer.is_empty() {
            // TODO: https://github.com/Sovereign-Labs/sovereign-sdk/issues/655
            // Gas metering suppose to prevent saving blobs from not allowed senders if they exit mid-slot
            let to_defer: Vec<&Da::BlobTransaction> = to_defer
                .iter()
                .filter(|b| {
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
                })
                .map(|b| &**b)
                .collect();
            self.store_blobs(current_slot, &to_defer, working_set)?
        }

        let num_blobs_requested = self
            .deferred_blobs_requested_for_execution_next_slot
            .get(working_set)
            .unwrap_or_default() as usize;
        let mut num_additional_blobs_to_process =
            num_blobs_requested.saturating_sub(past_deferred.len());

        // Continue running until we've either fulfilled the sequencer's request
        // or we've run out of deferred blobs
        while past_deferred.len() < num_additional_blobs_to_process
            && slot_for_next_blobs < current_slot
        {
            slot_for_next_blobs += 1;
            let mut blobs_from_next_slot =
                self.take_blobs_for_slot_height(slot_for_next_blobs, working_set);

            let num_blobs_from_next_slot = blobs_from_next_slot.len();

            // If the set of deferred blobs from the next slot in line contains more than the remainder needed to fill the request,
            //  we split that group and save the unused portion back into state
            if num_blobs_from_next_slot > num_additional_blobs_to_process {
                let blobs_to_process =
                    blobs_from_next_slot.split_off(num_additional_blobs_to_process as usize);
                past_deferred.extend(blobs_to_process.into_iter());
                self.store_blobs(
                    slot_for_next_blobs,
                    &blobs_from_next_slot.iter().collect::<Vec<_>>(),
                    working_set,
                )?;
            } else {
                past_deferred.extend(blobs_from_next_slot.into_iter())
            }

            // Update the count with the number of blobs processed
            num_additional_blobs_to_process =
                num_additional_blobs_to_process.saturating_sub(num_blobs_from_next_slot);
        }

        let non_priority_blobs = past_deferred.into_iter().map(Into::into);
        if !priority_blobs.is_empty() {
            Ok(priority_blobs
                .into_iter()
                .map(Into::into)
                .chain(non_priority_blobs)
                .collect())
        } else {
            // No blobs from preferred sequencer, nothing to save, older blobs have priority
            Ok(non_priority_blobs.collect())
        }
    }
}
