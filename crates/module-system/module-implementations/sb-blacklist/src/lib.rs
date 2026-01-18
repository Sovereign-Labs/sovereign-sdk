//! Blacklist module.
//!
//! This module defines the `Blacklist` Sovereign SDK module, which
//! manages a blacklist of wallet addresses.
//!
//! It exposes:
//! - Owner/manager configuration,
//! - Blacklist signer management,
//! - Per-wallet blacklist state (boolean),
//! - Helper methods for other modules (e.g. DEXes) to enforce that a
//!   wallet is NOT blacklisted.

mod call;
mod error;
mod event;
mod types;

pub use call::CallMessage;
pub use error::BlacklistError;
pub use event::Event;
pub use types::BlacklistConfig;

use sov_modules_api::{
    Context, EventEmitter, GenesisState, Module, ModuleId, ModuleInfo, ModuleRestApi, Spec,
    StateMap, StateValue, TxState,
};

/// Blacklist module definition.
///
/// This struct declares all on-chain state used by the blacklist:
/// - `owner`: address with ultimate control (can change the manager and toggle enforcement),
/// - `manager`: operational address that controls signers,
/// - `enforcement_enabled`: global flag to toggle enforcement checks,
/// - `blacklisted`: per-wallet blacklist status,
/// - `blacklist_signers`: addresses allowed to add/remove blacklist entries.
#[derive(Clone, ModuleInfo, ModuleRestApi)]
pub struct Blacklist<S: Spec> {
    #[id]
    pub id: ModuleId,

    /// Can change the manager and toggle enforcement.
    #[state]
    pub owner: StateValue<S::Address>,

    /// Operational address responsible for day-to-day configuration.
    #[state]
    pub manager: StateValue<S::Address>,

    /// Global flag controlling whether blacklist enforcement is active.
    #[state]
    pub enforcement_enabled: StateValue<bool>,

    /// Mapping from wallet address to its blacklist status.
    ///
    /// If a wallet is in the map with value `true`, it is blacklisted.
    /// Absence from the map or value `false` means not blacklisted.
    #[state]
    pub blacklisted: StateMap<S::Address, bool>,

    /// Mapping from address to whether it is allowed to act as a blacklist signer.
    #[state]
    pub blacklist_signers: StateMap<S::Address, bool>,
}

impl<S: Spec> Module for Blacklist<S> {
    type Spec = S;

    type Config = BlacklistConfig<S>;

    type CallMessage = CallMessage<S>;

    type Event = Event<S>;

    type Error = anyhow::Error;

    fn genesis(
        &mut self,
        _header: &<S::Da as sov_modules_api::DaSpec>::BlockHeader,
        config: &Self::Config,
        state: &mut impl GenesisState<S>,
    ) -> anyhow::Result<()> {
        self.owner.set(&config.owner, state)?;
        self.manager.set(&config.manager, state)?;
        self.enforcement_enabled
            .set(&config.enforcement_enabled, state)?;
        Ok(())
    }

    fn call(
        &mut self,
        msg: Self::CallMessage,
        context: &Context<Self::Spec>,
        state: &mut impl TxState<S>,
    ) -> Result<(), Self::Error> {
        call::execute(self, msg, context, state)
    }
}

impl<S: Spec> Blacklist<S> {
    // --- Public API for other modules (e.g., DEXes) ---

    /// Returns `true` if the wallet is currently blacklisted.
    pub fn is_blacklisted(
        &self,
        wallet: &S::Address,
        state: &mut impl TxState<S>,
    ) -> anyhow::Result<bool> {
        Ok(self.blacklisted.get(wallet, state)?.unwrap_or(false))
    }

    /// Require that the wallet is NOT blacklisted.
    ///
    /// Returns `Ok(())` if the wallet is not in the blacklist,
    /// or an error if it is blacklisted.
    ///
    /// If enforcement is disabled, this always returns `Ok(())`.
    pub fn enforce_not_blacklisted(
        &self,
        wallet: &S::Address,
        state: &mut impl TxState<S>,
    ) -> anyhow::Result<()> {
        // Skip enforcement if globally disabled
        if !self.enforcement_enabled.get(state)?.unwrap_or(true) {
            return Ok(());
        }

        if self.is_blacklisted(wallet, state)? {
            Err(BlacklistError::WalletBlacklisted.into())
        } else {
            Ok(())
        }
    }

    // --- Private helpers ---

    /// Returns `true` if the given sender is the configured manager.
    fn is_manager(&self, sender: &S::Address, state: &mut impl TxState<S>) -> anyhow::Result<bool> {
        let manager = self
            .manager
            .get(state)?
            .ok_or(BlacklistError::ManagerNotInitialized)?;

        Ok(sender == &manager)
    }

    /// Returns `true` if the given sender is the configured owner.
    fn is_owner(&self, sender: &S::Address, state: &mut impl TxState<S>) -> anyhow::Result<bool> {
        let owner = self
            .owner
            .get(state)?
            .ok_or(BlacklistError::OwnerNotInitialized)?;

        Ok(sender == &owner)
    }

    /// Returns `true` if the given address is configured as a blacklist signer.
    fn is_blacklist_signer(
        &self,
        signer: &S::Address,
        state: &mut impl TxState<S>,
    ) -> anyhow::Result<bool> {
        Ok(self.blacklist_signers.get(signer, state)?.unwrap_or(false))
    }

    /// Add or remove a wallet from the blacklist.
    fn write_blacklist(
        &mut self,
        wallet: &S::Address,
        blacklisted: bool,
        state: &mut impl TxState<S>,
    ) -> anyhow::Result<()> {
        if blacklisted {
            self.blacklisted.set(wallet, &true, state)?;
        } else {
            self.blacklisted.remove(wallet, state)?;
        }

        self.emit_event(
            state,
            Event::BlacklistSet {
                wallet: *wallet,
                blacklisted,
            },
        );

        Ok(())
    }
}
