use sov_modules_api::capabilities::{BlobRefOrOwned, BlobSelector};
use sov_modules_api::{Context, Spec};
use sov_rollup_interface::da::BlobReaderTrait;
use sov_state::WorkingSet;
use tracing::info;

use crate::BlobStorage;

impl<C: Context> BlobSelector for BlobStorage<C> {
    type Context = C;

    fn get_blobs_for_this_slot<'a, I, B>(
        &self,
        current_blobs: I,
        working_set: &mut WorkingSet<<Self::Context as Spec>::Storage>,
    ) -> anyhow::Result<Vec<BlobRefOrOwned<'a, B>>>
    where
        B: BlobReaderTrait,
        I: IntoIterator<Item = &'a mut B>,
    {
        // TODO: Chain-state module: https://github.com/Sovereign-Labs/sovereign-sdk/pull/598/
        let current_slot: u64 = self.get_current_slot_number(working_set);
        let past_deferred: Vec<B> = current_slot
            .checked_sub(self.get_deferred_slots_count(working_set))
            .map(|pull_from_slot| self.take_blobs_for_block_number(pull_from_slot, working_set))
            .unwrap_or_default();
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
        let mut to_defer: Vec<&mut B> = Vec::new();

        for blob in current_blobs {
            if blob.sender().as_ref() == &preferred_sequencer[..] {
                priority_blobs.push(blob);
            } else {
                to_defer.push(blob);
            }
        }

        // TODO: chain state module: https://github.com/Sovereign-Labs/sovereign-sdk/pull/598/
        self.slot_number.set(&(current_slot + 1), working_set);

        if !to_defer.is_empty() {
            // TODO: https://github.com/Sovereign-Labs/sovereign-sdk/issues/655
            // Gas metering suppose to prevent saving blobs from not allowed senders if they exit mid-slot
            let to_defer: Vec<&B> = to_defer
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

        if !priority_blobs.is_empty() {
            Ok(priority_blobs
                .into_iter()
                .map(Into::into)
                .chain(past_deferred.into_iter().map(Into::into))
                .collect())
        } else {
            // No blobs from preferred sequencer, nothing to save, older blobs have priority
            Ok(past_deferred.into_iter().map(Into::into).collect())
        }
    }
}
