//! Session registry module.
//!
//! This module defines the `SessionRegistry` Sovereign SDK module, which
//! manages per-wallet sessions for a single application running on an
//! app chain. It exposes:
//! - Owner/manager configuration,
//! - Session signer management,
//! - Per-wallet session state (expiry + bypass),
//! - Helper methods for other modules (e.g. DEXes) to enforce session
//!   presence and activeness.
//!
mod call;
mod error;
mod event;
mod types;

pub use call::CallMessage;
pub use error::SessionRegistryError;
pub use event::Event;
pub use types::{RegistryConfig, Session};

use sov_modules_api::da::Time;
use sov_modules_api::{
    Context, EventEmitter, GenesisState, Module, ModuleId, ModuleInfo, ModuleRestApi, Spec,
    StateMap, StateValue, TxState,
};

/// Session registry module definition.
///
/// This struct declares all on-chain state used by the registry:
/// - `owner`: address with ultimate control (can change the manager and toggle enforcement),
/// - `manager`: operational address that controls signers and bypass,
/// - `enforcement_enabled`: global flag to toggle enforcement checks,
/// - `sessions`: per-wallet session records,
/// - `session_signers`: addresses allowed to set/remove sessions.
#[derive(Clone, ModuleInfo, ModuleRestApi)]
pub struct SessionRegistry<S: Spec> {
    /// Unique identifier of this module in the runtime.
    #[id]
    pub id: ModuleId,

    /// Reference to the chain-state module (for time and other core data).
    #[module]
    pub chain_state: sov_chain_state::ChainState<S>,

    /// Address with ultimate ownership of the registry.
    ///
    /// Can change the manager and toggle enforcement.
    #[state]
    pub owner: StateValue<S::Address>,

    /// Operational address responsible for day-to-day configuration.
    ///
    /// The manager can set session signers and
    /// manages per-wallet bypass behavior.
    #[state]
    pub manager: StateValue<S::Address>,

    /// Global flag controlling whether session enforcement is active.
    #[state]
    pub enforcement_enabled: StateValue<bool>,

    /// Mapping from wallet address to its session state.
    #[state]
    pub sessions: StateMap<S::Address, Session>,

    /// Mapping from address to whether it is allowed to act as a session signer.
    #[state]
    pub session_signers: StateMap<S::Address, bool>,

    /// Offset to extend all session expiries by a fixed amount.
    /// Used in emergencies if backend services are down and need to extend sessions.
    #[state]
    pub expiry_offset: StateValue<i64>,
}

impl<S: Spec> Module for SessionRegistry<S> {
    type Spec = S;

    type Config = RegistryConfig<S>;

    type CallMessage = CallMessage<S>;

    type Event = Event<S>;

    type Error = anyhow::Error;

    /// Initialize module state at genesis.
    ///
    /// Values are taken from the [`RegistryConfig`] provided in the
    /// rollup’s genesis configuration.
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
        self.expiry_offset.set(&config.expiry_offset, state)?;
        Ok(())
    }

    /// Handle a single call message.
    ///
    /// This delegates to the `call::execute` function, which implements
    /// the routing and access control for all [`CallMessage`] variants.
    fn call(
        &mut self,
        msg: Self::CallMessage,
        context: &Context<Self::Spec>,
        state: &mut impl TxState<S>,
    ) -> Result<(), Self::Error> {
        call::execute(self, msg, context, state)
    }
}

impl<S: Spec> SessionRegistry<S> {
    /// --- APIs for DEX modules ---

    /// Returns `true` if the wallet currently has an active session.
    ///
    /// A session is considered active if:
    /// - `bypass` is set to `true`, or
    /// - `effective_expiry` (which includes offset) is strictly greater than the current
    ///   chain time.
    pub fn is_session_active(
        &self,
        wallet: &S::Address,
        state: &mut impl TxState<S>,
    ) -> anyhow::Result<bool> {
        if let Some(session) = self.sessions.get(wallet, state)? {
            if session.bypass {
                return Ok(true);
            }

            let effective_expiry_ts =
                session.expiry_ts + self.expiry_offset.get(state)?.unwrap_or(0);

            let now: Time = self.chain_state.get_time(state)?;
            let now_ts = now.secs();

            if effective_expiry_ts > now_ts {
                return Ok(true);
            }
        }

        Ok(false)
    }

    /// Require that the wallet has an active session.
    ///
    /// Returns `Ok(())` if the session is active according to
    /// [`is_session_active`], or an error otherwise.
    pub fn enforce_session_active(
        &self,
        wallet: &S::Address,
        state: &mut impl TxState<S>,
    ) -> anyhow::Result<()> {
        if !self.enforcement_enabled.get(state)?.unwrap_or(true) {
            return Ok(());
        }

        if self.is_session_active(wallet, state)? {
            Ok(())
        } else {
            Err(SessionRegistryError::SessionNotActive.into())
        }
    }

    /// Returns `true` if a session is present (i.e. not deleted) for a wallet.
    ///
    /// A session is present if:
    /// - It exists and has `bypass == true`, or
    /// - It exists and `expiry_ts != 0`.
    pub fn is_session_present(
        &self,
        wallet: &S::Address,
        state: &mut impl TxState<S>,
    ) -> anyhow::Result<bool> {
        let session_opt = self.sessions.get(wallet, state)?;

        Ok(match session_opt {
            Some(session) => session.bypass || session.expiry_ts != 0,
            None => false,
        })
    }

    /// Require that a session is present (i.e. not deleted) for a wallet.
    ///
    /// Returns `Ok(())` if a session is present according to
    /// [`is_session_present`], or an error otherwise.
    pub fn enforce_session_present(
        &self,
        wallet: &S::Address,
        state: &mut impl TxState<S>,
    ) -> anyhow::Result<()> {
        if !self.enforcement_enabled.get(state)?.unwrap_or(true) {
            return Ok(());
        }

        if self.is_session_present(wallet, state)? {
            Ok(())
        } else {
            Err(SessionRegistryError::SessionNotPresent.into())
        }
    }

    /// --- Helpers ---

    /// Returns `true` if the given sender is the configured manager.
    ///
    /// # Errors
    ///
    /// - Returns an error if the manager has not been initialized in state.
    fn is_manager(&self, sender: &S::Address, state: &mut impl TxState<S>) -> anyhow::Result<bool> {
        let manager = self
            .manager
            .get(state)?
            .ok_or(SessionRegistryError::ManagerNotInitialized)?;

        Ok(sender == &manager)
    }

    /// Returns `true` if the given sender is the configured owner.
    ///
    /// # Errors
    ///
    /// - Returns an error if the owner has not been initialized in state.
    fn is_owner(&self, sender: &S::Address, state: &mut impl TxState<S>) -> anyhow::Result<bool> {
        let owner = self
            .owner
            .get(state)?
            .ok_or(SessionRegistryError::OwnerNotInitialized)?;

        Ok(sender == &owner)
    }

    /// Returns `true` if the given address is configured as a session signer.
    ///
    /// Absence in the map is treated as `false`.
    fn is_session_signer(
        &self,
        signer: &S::Address,
        state: &mut impl TxState<S>,
    ) -> anyhow::Result<bool> {
        Ok(self.session_signers.get(signer, state)?.unwrap_or(false))
    }

    /// Create, update, or delete the session for a wallet.
    ///
    /// - If `expires_at == 0`, the session is removed.
    /// - Otherwise, a new `Session` is written with expiry_ts = expires_at
    ///   and `bypass` either retained from any existing session or set to
    ///   `false` if none exists.
    fn write_session(
        &mut self,
        wallet: &S::Address,
        expires_at: i64,
        state: &mut impl TxState<S>,
    ) -> anyhow::Result<()> {
        if expires_at == 0 {
            self.sessions.remove(wallet, state)?;

            self.emit_event(
                state,
                Event::SessionSet {
                    wallet: wallet.clone(),
                    expiry_ts: 0,
                },
            );
        } else {
            // retain existing bypass flag if any
            let existing = self.sessions.get(wallet, state)?;
            let bypass = existing.map(|s| s.bypass).unwrap_or(false);

            let session = Session {
                expiry_ts: expires_at,
                bypass,
            };

            self.sessions.set(wallet, &session, state)?;

            self.emit_event(
                state,
                Event::SessionSet {
                    wallet: wallet.clone(),
                    expiry_ts: expires_at,
                },
            );
        }

        Ok(())
    }
}
