use sov_modules_api::hooks::{ApplyBlobHooks, SlotHooks, TxHooks};
use sov_modules_api::transaction::Transaction;
use sov_modules_api::{Context, Spec};
use sov_modules_stf_template::SequencerOutcome;
#[cfg(feature = "experimental")]
use sov_rollup_interface::da::BlockHeaderTrait;
use sov_rollup_interface::da::{BlobReaderTrait, DaSpec};
use sov_sequencer_registry::SequencerRegistry;
use sov_state::{AccessoryWorkingSet, WorkingSet};
use tracing::info;

use crate::runtime::Runtime;

impl<C: Context, Da: DaSpec> TxHooks for Runtime<C, Da> {
    type Context = C;

    fn pre_dispatch_tx_hook(
        &self,
        tx: &Transaction<Self::Context>,
        working_set: &mut WorkingSet<<Self::Context as Spec>::Storage>,
    ) -> anyhow::Result<<Self::Context as Spec>::Address> {
        self.accounts.pre_dispatch_tx_hook(tx, working_set)
    }

    fn post_dispatch_tx_hook(
        &self,
        tx: &Transaction<Self::Context>,
        working_set: &mut WorkingSet<<Self::Context as Spec>::Storage>,
    ) -> anyhow::Result<()> {
        self.accounts.post_dispatch_tx_hook(tx, working_set)
    }
}

impl<C: Context, Da: DaSpec> ApplyBlobHooks<Da::BlobTransaction> for Runtime<C, Da> {
    type Context = C;
    type BlobResult =
        SequencerOutcome<<<Da as DaSpec>::BlobTransaction as BlobReaderTrait>::Address>;

    fn begin_blob_hook(
        &self,
        blob: &mut Da::BlobTransaction,
        working_set: &mut WorkingSet<<Self::Context as Spec>::Storage>,
    ) -> anyhow::Result<()> {
        self.sequencer_registry.begin_blob_hook(blob, working_set)
    }

    fn end_blob_hook(
        &self,
        result: Self::BlobResult,
        working_set: &mut WorkingSet<<Self::Context as Spec>::Storage>,
    ) -> anyhow::Result<()> {
        match result {
            SequencerOutcome::Rewarded(_reward) => {
                // TODO: Process reward here or above.
                <SequencerRegistry<C> as ApplyBlobHooks<Da::BlobTransaction>>::end_blob_hook(
                    &self.sequencer_registry,
                    sov_sequencer_registry::SequencerOutcome::Completed,
                    working_set,
                )
            }
            SequencerOutcome::Ignored => Ok(()),
            SequencerOutcome::Slashed {
                reason,
                sequencer_da_address,
            } => {
                info!("Sequencer {} slashed: {:?}", sequencer_da_address, reason);
                <SequencerRegistry<C> as ApplyBlobHooks<Da::BlobTransaction>>::end_blob_hook(
                    &self.sequencer_registry,
                    sov_sequencer_registry::SequencerOutcome::Slashed {
                        sequencer: sequencer_da_address.as_ref().to_vec(),
                    },
                    working_set,
                )
            }
        }
    }
}

impl<C: Context, Da: DaSpec> SlotHooks<Da> for Runtime<C, Da> {
    type Context = C;

    fn begin_slot_hook(
        &self,
        #[allow(unused_variables)] slot_header: &Da::BlockHeader,
        #[allow(unused_variables)] validity_condition: &Da::ValidityCondition,
        #[allow(unused_variables)] working_set: &mut sov_state::WorkingSet<
            <Self::Context as Spec>::Storage,
        >,
    ) {
        #[cfg(feature = "experimental")]
        self.evm
            .begin_slot_hook(slot_header.hash().into(), working_set);
    }

    fn end_slot_hook(
        &self,
        #[allow(unused_variables)] working_set: &mut sov_state::WorkingSet<
            <Self::Context as Spec>::Storage,
        >,
    ) {
        #[cfg(feature = "experimental")]
        self.evm.end_slot_hook(working_set);
    }

    fn finalize_slot_hook(
        &self,
        #[allow(unused_variables)] root_hash: [u8; 32],
        #[allow(unused_variables)] accesorry_working_set: &mut AccessoryWorkingSet<
            <Self::Context as Spec>::Storage,
        >,
    ) {
        #[cfg(feature = "experimental")]
        self.evm
            .finalize_slot_hook(root_hash, accesorry_working_set);
    }
}
