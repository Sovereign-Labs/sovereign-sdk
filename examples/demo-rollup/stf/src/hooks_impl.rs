use sov_accounts::AccountsTxHook;
use sov_bank::BankTxHook;
use sov_modules_api::hooks::{ApplyBlobHooks, FinalizeHook, SlotHooks, TxHooks};
use sov_modules_api::transaction::Transaction;
use sov_modules_api::{AccessoryWorkingSet, Context, Spec, WorkingSet};
use sov_modules_stf_blueprint::{RuntimeTxHook, SequencerOutcome};
#[cfg(feature = "experimental")]
use sov_rollup_interface::da::BlockHeaderTrait;
use sov_rollup_interface::da::{BlobReaderTrait, DaSpec};
use sov_sequencer_registry::SequencerRegistry;
use sov_state::Storage;
use tracing::info;

use crate::runtime::Runtime;

impl<C: Context, Da: DaSpec> TxHooks for Runtime<C, Da> {
    type Context = C;
    type PreArg = RuntimeTxHook<C>;
    type PreResult = C;

    fn pre_dispatch_tx_hook(
        &self,
        tx: &Transaction<Self::Context>,
        working_set: &mut WorkingSet<C>,
        arg: &RuntimeTxHook<C>,
    ) -> anyhow::Result<C> {
        let RuntimeTxHook { height, sequencer } = arg;
        let AccountsTxHook { sender, sequencer } =
            self.accounts
                .pre_dispatch_tx_hook(tx, working_set, sequencer)?;

        let hook = BankTxHook { sender, sequencer };
        self.bank.pre_dispatch_tx_hook(tx, working_set, &hook)?;

        Ok(C::new(hook.sender, hook.sequencer, *height))
    }

    fn post_dispatch_tx_hook(
        &self,
        tx: &Transaction<Self::Context>,
        ctx: &C,
        working_set: &mut WorkingSet<C>,
    ) -> anyhow::Result<()> {
        self.accounts.post_dispatch_tx_hook(tx, ctx, working_set)?;
        self.bank.post_dispatch_tx_hook(tx, ctx, working_set)?;
        Ok(())
    }
}

impl<C: Context, Da: DaSpec> ApplyBlobHooks<Da::BlobTransaction> for Runtime<C, Da> {
    type Context = C;
    type BlobResult =
        SequencerOutcome<<<Da as DaSpec>::BlobTransaction as BlobReaderTrait>::Address>;

    fn begin_blob_hook(
        &self,
        blob: &mut Da::BlobTransaction,
        working_set: &mut WorkingSet<C>,
    ) -> anyhow::Result<()> {
        // Before executing each batch, check that the sender is registered as a sequencer
        self.sequencer_registry.begin_blob_hook(blob, working_set)
    }

    fn end_blob_hook(
        &self,
        result: Self::BlobResult,
        working_set: &mut WorkingSet<C>,
    ) -> anyhow::Result<()> {
        match result {
            SequencerOutcome::Rewarded(_reward) => {
                // TODO: Process reward here or above.
                <SequencerRegistry<C, Da> as ApplyBlobHooks<Da::BlobTransaction>>::end_blob_hook(
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
                <SequencerRegistry<C, Da> as ApplyBlobHooks<Da::BlobTransaction>>::end_blob_hook(
                    &self.sequencer_registry,
                    sov_sequencer_registry::SequencerOutcome::Slashed {
                        sequencer: sequencer_da_address,
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
        #[allow(unused_variables)]
        pre_state_root: &<<Self::Context as Spec>::Storage as Storage>::Root,
        #[allow(unused_variables)] working_set: &mut sov_modules_api::WorkingSet<C>,
    ) {
        #[cfg(feature = "experimental")]
        self.evm
            .begin_slot_hook(slot_header.hash().into(), pre_state_root, working_set);
    }

    fn end_slot_hook(
        &self,
        #[allow(unused_variables)] working_set: &mut sov_modules_api::WorkingSet<C>,
    ) {
        #[cfg(feature = "experimental")]
        self.evm.end_slot_hook(working_set);
    }
}

impl<C: Context, Da: sov_modules_api::DaSpec> FinalizeHook<Da> for Runtime<C, Da> {
    type Context = C;

    fn finalize_hook(
        &self,
        #[allow(unused_variables)] root_hash: &<<Self::Context as Spec>::Storage as Storage>::Root,
        #[allow(unused_variables)] accessory_working_set: &mut AccessoryWorkingSet<C>,
    ) {
        #[cfg(feature = "experimental")]
        self.evm.finalize_hook(root_hash, accessory_working_set);
    }
}
