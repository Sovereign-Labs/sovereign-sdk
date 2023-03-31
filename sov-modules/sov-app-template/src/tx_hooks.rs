use crate::tx_verifier::{Transaction, VerifiedTx};
use sov_modules_api::{Context, Spec};
use sov_state::WorkingSet;

/// TxHooks allows injecting custom logic into a transaction processing pipeline.
pub trait TxHooks {
    type Context: Context;

    /// pre_dispatch_tx_hook runs just before a transaction is dispatched to an appropriate module.
    fn pre_dispatch_tx_hook(
        &mut self,
        tx: Transaction<Self::Context>,
        working_set: &mut WorkingSet<<Self::Context as Spec>::Storage>,
    ) -> anyhow::Result<VerifiedTx<Self::Context>>;

    /// post_dispatch_tx_hook runs after the tx is dispatched to an appropriate module.
    fn post_dispatch_tx_hook(
        &mut self,
        tx: VerifiedTx<Self::Context>,
        working_set: &mut WorkingSet<<Self::Context as Spec>::Storage>,
    );
}
