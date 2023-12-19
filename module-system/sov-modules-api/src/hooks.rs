use sov_modules_core::{AccessoryWorkingSet, Context, Spec, Storage, WorkingSet};
use sov_rollup_interface::da::{BlobReaderTrait, DaSpec};

use crate::transaction::Transaction;

/// Hooks that execute within the `StateTransitionFunction::apply_blob` function for each processed transaction.
///
/// The arguments consist of expected concretely implemented associated types for the hooks. At
/// runtime, compatible implementations are selected and utilized by the system to construct its
/// setup procedures and define post-execution routines.
pub trait TxHooks {
    type Context: Context;

    /// # Additional context
    ///
    /// To prevent cloning of the `PreArg` object even when passed as owned, an affordable
    /// alternative is to utilize [std::rc::Rc] for owning the underlying data. Subsequently,
    /// acquire its owned values once the hooks have been executed.
    ///
    /// [std::rc::Rc], in comparison to [std::sync::Arc], tends to be faster since the dispatch
    /// process does not occur within this `sync` context. Consequently, we are relieved from
    /// dealing with thread synchronization concerns. Although this performance improvement
    /// primarily pertains to write operations - which infrequently occur on hook arguments - the
    /// resulting trade-off is worthwhile without additional cost. For further details, please
    /// refer to https://spcl.inf.ethz.ch/Publications/.pdf/atomic-bench.pdf.
    type PreArg;
    type PreResult;

    /// Runs just before a transaction is dispatched to an appropriate module.
    fn pre_dispatch_tx_hook(
        &self,
        tx: &Transaction<Self::Context>,
        working_set: &mut WorkingSet<Self::Context>,
        arg: Self::PreArg,
    ) -> anyhow::Result<Self::PreResult>;

    /// Runs after the tx is dispatched to an appropriate module.
    /// IF this hook returns error rollup panics
    fn post_dispatch_tx_hook(
        &self,
        tx: &Transaction<Self::Context>,
        ctx: &Self::Context,
        working_set: &mut WorkingSet<Self::Context>,
    ) -> anyhow::Result<()>;
}

/// Hooks related to the Sequencer functionality.
/// In essence, the sequencer locks a bond at the beginning of the `StateTransitionFunction::apply_blob`,
/// and is rewarded once a blob of transactions is processed.
pub trait ApplyBlobHooks<B: BlobReaderTrait> {
    type Context: Context;
    type BlobResult;

    /// Runs at the beginning of apply_blob, locks the sequencer bond.
    /// If this hook returns Err, batch is not applied
    fn begin_blob_hook(
        &self,
        blob: &mut B,
        working_set: &mut WorkingSet<Self::Context>,
    ) -> anyhow::Result<()>;

    /// Executes at the end of apply_blob and rewards or slashed the sequencer
    /// If this hook returns Err rollup panics
    fn end_blob_hook(
        &self,
        result: Self::BlobResult,
        working_set: &mut WorkingSet<Self::Context>,
    ) -> anyhow::Result<()>;
}

/// Hooks that execute during the `StateTransitionFunction::begin_slot` and `end_slot` functions.
pub trait SlotHooks<Da: DaSpec> {
    type Context: Context;

    fn begin_slot_hook(
        &self,
        slot_header: &Da::BlockHeader,
        validity_condition: &Da::ValidityCondition,
        pre_state_root: &<<Self::Context as Spec>::Storage as Storage>::Root,
        working_set: &mut WorkingSet<Self::Context>,
    );

    fn end_slot_hook(&self, working_set: &mut WorkingSet<Self::Context>);
}

pub trait FinalizeHook<Da: DaSpec> {
    type Context: Context;

    fn finalize_hook(
        &self,
        root_hash: &<<Self::Context as Spec>::Storage as Storage>::Root,
        accessory_working_set: &mut AccessoryWorkingSet<Self::Context>,
    );
}
