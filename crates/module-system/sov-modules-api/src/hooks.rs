use sov_state::Storage;

use crate::transaction::AuthenticatedTransactionData;
#[cfg(feature = "native")]
use crate::AccessoryStateReaderAndWriter;
use crate::{Context, Module, Spec, StateCheckpoint, TxState};

/// Hooks that execute within the `StateTransitionFunction::apply_blob` function for each processed transaction.
///
/// If the hook returns an error, the transaction is reverted. Note that unlike `BlockHooks`, `TxHooks` are metered,
/// so you don't need to take special care not to consume to many resources.
pub trait TxHooks {
    /// The [`Spec`] of the runtime, which defines the relevant types
    type Spec: Spec;

    /// Runs just before a transaction is dispatched to an appropriate module.
    /// If this hook returns an error, the transaction is reverted.
    fn pre_dispatch_tx_hook<T: TxState<Self::Spec>>(
        &mut self,
        _tx: &AuthenticatedTransactionData<Self::Spec>,
        _state: &mut T,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    /// Runs after the tx is dispatched to an appropriate module.
    /// If this hook returns an error, the transaction is reverted.
    fn post_dispatch_tx_hook<T: TxState<Self::Spec>>(
        &mut self,
        _tx: &AuthenticatedTransactionData<Self::Spec>,
        _ctx: &Context<Self::Spec>,
        _state: &mut T,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}

/// Autoref blanket implementation of the [`TxHooks`] trait.
/// Any module can override the default behavior by implementing the [`TxHooks`] trait.
impl<T: Module> TxHooks for &mut T {
    type Spec = T::Spec;

    fn pre_dispatch_tx_hook<S: TxState<Self::Spec>>(
        &mut self,
        _tx: &AuthenticatedTransactionData<Self::Spec>,
        _state: &mut S,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn post_dispatch_tx_hook<S: TxState<Self::Spec>>(
        &mut self,
        _tx: &AuthenticatedTransactionData<Self::Spec>,
        _ctx: &Context<Self::Spec>,
        _state: &mut S,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}

/// Hooks that execute at the beginning and end of each rollup block.
///
/// A rollup block is created at the discretion of the blob-selector -
/// on "based" rollups, this happens every time the DA layer produces a new block, regardless of the number of batches.
/// On "soft-confirmed" rollups, this happens each time the preferred sequencer successfully lands its next batch on the DA layer.
///
/// # Important
/// Note that hook execution is not metered, so be careful about using expensive operations in your hooks or allowing
/// users to arbitrarily increase the execution time of your module hooks. Failure to follow this rule will result in
/// poor performance or potential DoS attacks.
///
/// ## Examples
/// - (Safe): Iterate over a fixed number of items - for example, update oracle prices on a fixed number of assets
/// - (Safe): Iterate over a number of items that is limited per-block. For example, iterate over all accounts that were modified last block.
/// - (Dangerous): Iterate over an arbitrary number of items - for example, iterate over all accounts on the rollup.
pub trait BlockHooks {
    /// The runtime spec.
    type Spec: Spec;

    /// Runs at the beginning of each rollup block. See trait description for important performance considerations.
    ///
    /// ## Visible Hash
    /// The `visible_hash` argument passed to this hook is a rollup state root suitable for making storage proofs.
    /// Recall that the rollup state root has two components: a "user space" state root where normal modules store their state, and a "kernel space" state root
    /// where information from the DA layer is stored as it comes in.
    ///
    /// The *user* space state root passed to this hook is simply the pre-state root of the `N-STATE_ROOT_DELAY_BLOCKS`th rollup block.
    /// The *kernel* space state root is the pre-state root of the visible slot number associated with `N-STATE_ROOT_DELAY_BLOCKS` block.
    /// Suppose `STATE_ROOT_DELAY_BLOCKS` = 0. Then the state roots passed are simply the
    /// pre-state roots of the current block.
    fn begin_rollup_block_hook(
        &mut self,
        _visible_hash: &<<Self::Spec as Spec>::Storage as Storage>::Root,
        _state: &mut StateCheckpoint<Self::Spec>,
    ) {
    }

    /// Hook that runs at the end of block execution. See trait description for details and important performance considerations.
    fn end_rollup_block_hook(&mut self, _state: &mut StateCheckpoint<Self::Spec>) {}
}

/// Autoref blanket implementation of the [`BlockHooks`] trait.
/// Any module can override the default behavior by implementing the [`BlockHooks`] trait.
impl<T: Module> BlockHooks for &mut T {
    type Spec = T::Spec;

    fn begin_rollup_block_hook(
        &mut self,
        _visible_hash: &<<Self::Spec as Spec>::Storage as Storage>::Root,
        _state: &mut StateCheckpoint<Self::Spec>,
    ) {
    }

    fn end_rollup_block_hook(&mut self, _state: &mut StateCheckpoint<Self::Spec>) {}
}

/// A hook that runs after all state changes from a block have been computed.
/// See the docs on `FinalizeHook::finalize_hook` for more details and important performance considerations.
#[cfg(feature = "native")]
pub trait FinalizeHook {
    /// The runtime spec.
    type Spec: Spec;

    /// Hook that defines logic that runs after all state changes from a block have been computed.
    /// At this point, it is impossible to alter state variables because the state root is fixed.
    /// However, non-state data can be modified.
    /// Use this hook to perform any post-processing changes to the accessory state.
    ///
    /// Note that the finalize hook does not run as part of zk-proving. (In other words, it's always gated
    /// by the `native` feature flag). This is safe, because we only allow the finalize hook to accessory state,
    /// which can't be read by transactions.
    ///
    /// Note that hook finalize execution is not metered, so be careful about using expensive operations in your hooks or allowing
    /// users to arbitrarily increase the execution time of your module hooks. Failure to follow this rule will result in
    /// poor performance or potential DoS attacks.
    ///
    /// ## Examples
    /// - (Safe): Iterate over a fixed number of items - for example, update oracle prices on a fixed number of assets
    /// - (Safe): Iterate over a number of items that is limited per-block. For example, iterate over all accounts that were modified last block.
    /// - (Dangerous): Iterate over an arbitrary number of items - for example, iterate over all accounts on the rollup.
    fn finalize_hook(
        &mut self,
        _root_hash: &<<Self::Spec as Spec>::Storage as Storage>::Root,
        _state: &mut impl AccessoryStateReaderAndWriter,
    ) {
    }
}

/// Autoref blanket implementation of the [`FinalizeHook`] trait.
/// Any module can override the default behavior by implementing the [`FinalizeHook`] trait.
#[cfg(feature = "native")]
impl<T: Module> FinalizeHook for &mut T {
    type Spec = T::Spec;

    fn finalize_hook(
        &mut self,
        _root_hash: &<<Self::Spec as Spec>::Storage as Storage>::Root,
        _state: &mut impl AccessoryStateReaderAndWriter,
    ) {
    }
}
