//! Defines the CallMessages accepted by the blob storage module

use sov_modules_api::prelude::*;
use sov_modules_api::{Context, DaSpec, WorkingSet};

use crate::BlobStorage;

/// A call message for the blob storage module
#[cfg_attr(
    feature = "native",
    derive(sov_modules_api::macros::CliWalletArg),
    derive(schemars::JsonSchema)
)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize),
    derive(serde::Deserialize)
)]
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq, Clone)]
pub enum CallMessage {
    /// Asks the blob selector to process up to the given number of deferred blobs early.
    /// Only the preferred sequencer may send this message.
    ProcessDeferredBlobsEarly {
        /// The number of blobs to process early
        number: u16,
    },
}

impl<C: Context, Da: DaSpec> BlobStorage<C, Da> {
    pub(crate) fn handle_process_blobs_early_msg(
        &self,
        context: &C,
        number: u16,
        working_set: &mut WorkingSet<C>,
    ) {
        if let Some(preferred_sequencer) = self
            .sequencer_registry
            .get_preferred_sequencer_rollup_address(working_set)
        {
            if context.sender() == &preferred_sequencer {
                self.deferred_blobs_requested_for_execution_next_slot
                    .set(&number, working_set);
            }
        }
    }
}
