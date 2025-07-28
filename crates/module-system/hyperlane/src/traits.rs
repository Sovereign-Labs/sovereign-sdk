//! Traits for the hyperlane protocol.
use sov_bank::Amount;
use sov_modules_api::{Context, HexHash, HexString, Spec, TxState};

use crate::types::HookType;
use crate::Message;

/// Allows a module to be used as a post-dispatch hook.
pub trait PostDispatchHook<S: Spec> {
    /// Get the hook type for a given address. Used by the relayer to determine which metadata to include with its message.
    fn hook_type(&self, addr: &S::Address, state: &mut impl TxState<S>)
        -> anyhow::Result<HookType>;

    /// Check if the hook supports metadata.
    fn supports_metadata(
        &self,
        metadata: &HexString,
        state: &mut impl TxState<S>,
    ) -> anyhow::Result<bool>;

    /// Post-dispatch hook. Called by the mailbox at the end of the `dispatch` method.
    #[allow(clippy::too_many_arguments)]
    fn post_dispatch(
        &mut self,
        message_id: &HexHash,
        message: &Message,
        metadata: &HexString,
        relayer: &S::Address,
        gas_payment_limit: Amount,
        context: &Context<S>,
        state: &mut impl TxState<S>,
    ) -> anyhow::Result<()>;

    /// Estimate the cost of dispatch, in the native currency of the chain.
    fn quote_dispatch(
        &self,
        message: &Message,
        metadata: &HexString,
        relayer: &S::Address,
        context: &Context<S>,
        state: &mut impl TxState<S>,
    ) -> anyhow::Result<Amount>;
}

/// A post-dispatch hook that does nothing.
pub enum NoOpPostDispatchHook {}

impl<S: Spec> PostDispatchHook<S> for NoOpPostDispatchHook {
    fn hook_type(
        &self,
        _addr: &S::Address,
        _state: &mut impl TxState<S>,
    ) -> anyhow::Result<HookType> {
        Ok(HookType::Unused)
    }

    fn supports_metadata(
        &self,
        _metadata: &HexString,
        _state: &mut impl TxState<S>,
    ) -> anyhow::Result<bool> {
        Ok(false)
    }

    fn post_dispatch(
        &mut self,
        _message_id: &HexHash,
        _message: &Message,
        _metadata: &HexString,
        _relayer: &S::Address,
        _gas_payment_limit: Amount,
        _context: &Context<S>,
        _state: &mut impl TxState<S>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn quote_dispatch(
        &self,
        _message: &Message,
        _metadata: &HexString,
        _relayer: &S::Address,
        _context: &Context<S>,
        _state: &mut impl TxState<S>,
    ) -> anyhow::Result<Amount> {
        Ok(Amount::ZERO)
    }
}
