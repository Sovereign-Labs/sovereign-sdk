use sov_rollup_interface::da::{BlobReaderTrait, DaSpec};
use sov_rollup_interface::services::da::SlotData;
use sov_state::WorkingSet;

use crate::transaction::Transaction;
use crate::{Context, Spec};

/// Hooks that execute within the `StateTransitionFunction::apply_blob` function for each processed transaction.
pub trait TxHooks {
    type Context: Context;

    /// Runs just before a transaction is dispatched to an appropriate module.
    /// TODO: Why does it return address?
    /// Does it implies that it should do signature verification.
    /// Can other code rely on that assumption?
    fn pre_dispatch_tx_hook(
        &self,
        tx: &Transaction<Self::Context>,
        working_set: &mut WorkingSet<<Self::Context as Spec>::Storage>,
    ) -> anyhow::Result<<Self::Context as Spec>::Address>;

    /// Runs after the tx is dispatched to an appropriate module.
    /// IF this hook returns error rollup panics
    fn post_dispatch_tx_hook(
        &self,
        tx: &Transaction<Self::Context>,
        working_set: &mut WorkingSet<<Self::Context as Spec>::Storage>,
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
        working_set: &mut WorkingSet<<Self::Context as Spec>::Storage>,
    ) -> anyhow::Result<()>;

    /// Executes at the end of apply_blob and rewards or slashed the sequencer
    /// If this hook returns Err rollup panics
    fn end_blob_hook(
        &self,
        result: Self::BlobResult,
        working_set: &mut WorkingSet<<Self::Context as Spec>::Storage>,
    ) -> anyhow::Result<()>;
}

/// Hooks that execute during the `StateTransitionFunction::begin_slot` and `end_slot` functions.
pub trait SlotHooks<Da: DaSpec> {
    type Context: Context;

    fn begin_slot_hook(
        &self,
        slot_data: &impl SlotData<Cond = Da::ValidityCondition>,
        working_set: &mut WorkingSet<<Self::Context as Spec>::Storage>,
    );

    fn end_slot_hook(
        &self,
        root_hash: [u8; 32],
        working_set: &mut WorkingSet<<Self::Context as Spec>::Storage>,
    );
}
