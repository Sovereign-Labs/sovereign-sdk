//! Call messages and execution entrypoint for the `SessionRegistry` module.

use schemars::JsonSchema;
use sov_modules_api::macros::serialize;
use sov_modules_api::macros::UniversalWallet;
use sov_modules_api::{Context, EventEmitter, Spec, TxState};

use crate::{Event, Session, SessionRegistry, SessionRegistryError};

/// Transaction-level messages supported by the `SessionRegistry`.
///
/// Access control is enforced in [`execute`]:
/// - `SetManager`: owner-only
/// - `SetEnforcementEnabled`: owner-only
/// - `SetSessionSigner`: manager-only
/// - `SetSession` / `SetSessionBatch`: session-signer-only
/// - `SetBypass`: manager-only
/// - `SetExpiryOffset`: owner-only
#[derive(Debug, Clone, PartialEq, Eq, JsonSchema, UniversalWallet)]
#[serialize(Borsh, Serde)]
#[serde(rename_all = "snake_case")]
#[schemars(bound = "S: Spec", rename = "CallMessage")]
pub enum CallMessage<S: Spec> {
    /// Update the manager address.
    SetManager { new_manager: S::Address },

    /// Enable or disable global session enforcement.
    SetEnforcementEnabled { enabled: bool },

    /// Grant or revoke session-signer privileges for an address.
    SetSessionSigner { signer: S::Address, allowed: bool },

    /// Set or delete the session for a single wallet.
    ///
    /// `expires_at == 0` removes the session (see `write_session`).
    SetSession { wallet: S::Address, expires_at: i64 },

    /// Set or delete sessions for a batch of wallets.
    SetSessionBatch {
        wallets: Vec<S::Address>,
        expiries: Vec<i64>,
    },

    /// Set or clear the bypass flag for a wallet.
    ///
    /// When `bypass == true`, the wallet is always treated as having
    /// an active and present session.
    SetBypass { wallet: S::Address, bypass: bool },

    /// Assert that a wallet has an active session.
    EnforceSessionActive { wallet: S::Address },

    /// Assert that a wallet has a present (non-deleted) session.
    EnforceSessionPresent { wallet: S::Address },

    /// Set a new global expiry offset.
    SetExpiryOffset { new_offset: i64 },
}

/// Route a CallMessage to the corresponding `SessionRegistry` logic.
///
/// This is the main entrypoint used by the runtime:
/// it applies access control based on `context.sender()` and updates
/// module state, emitting events where appropriate.
pub fn execute<S: Spec>(
    module: &mut SessionRegistry<S>,
    msg: CallMessage<S>,
    context: &Context<S>,
    state: &mut impl TxState<S>,
) -> anyhow::Result<()> {
    match msg {
        CallMessage::SetManager { new_manager } => {
            if !module.is_owner(context.sender(), state)? {
                return Err(SessionRegistryError::UnauthorizedOwner.into());
            }

            let old_manager = module.manager.get(state)?;

            module.manager.set(&new_manager, state)?;

            module.emit_event(
                state,
                Event::ManagerSet {
                    old_manager,
                    new_manager,
                },
            );

            Ok(())
        }
        CallMessage::SetEnforcementEnabled { enabled } => {
            if !module.is_owner(context.sender(), state)? {
                return Err(SessionRegistryError::UnauthorizedOwner.into());
            }

            module.enforcement_enabled.set(&enabled, state)?;

            module.emit_event(state, Event::EnforcementEnabledSet { enabled });

            Ok(())
        }
        CallMessage::SetSessionSigner { signer, allowed } => {
            if !module.is_manager(context.sender(), state)? {
                return Err(SessionRegistryError::UnauthorizedManager.into());
            }

            module.session_signers.set(&signer, &allowed, state)?;

            module.emit_event(state, Event::SessionSignerSet { signer, allowed });

            Ok(())
        }
        CallMessage::SetSession { wallet, expires_at } => {
            if !module.is_session_signer(context.sender(), state)? {
                return Err(SessionRegistryError::UnauthorizedSessionSigner.into());
            }

            module.write_session(&wallet, expires_at, state)?;

            Ok(())
        }
        CallMessage::SetSessionBatch { wallets, expiries } => {
            if !module.is_session_signer(context.sender(), state)? {
                return Err(SessionRegistryError::UnauthorizedSessionSigner.into());
            }

            if wallets.len() != expiries.len() {
                return Err(SessionRegistryError::InvalidBatchLengths.into());
            }

            for (wallet, expires_at) in wallets.iter().zip(expiries.iter().copied()) {
                module.write_session(wallet, expires_at, state)?;
            }

            Ok(())
        }
        CallMessage::SetBypass { wallet, bypass } => {
            if !module.is_manager(context.sender(), state)? {
                return Err(SessionRegistryError::UnauthorizedManager.into());
            }

            let maybe_session = module.sessions.get(&wallet, state)?;

            match maybe_session {
                None => {
                    if !bypass {
                        return Ok(());
                    }

                    let session = Session {
                        expiry_ts: 0,
                        bypass: true,
                    };

                    module.sessions.set(&wallet, &session, state)?;
                }
                Some(mut session) => {
                    if session.expiry_ts == 0 && !bypass {
                        module.sessions.remove(&wallet, state)?;
                    } else {
                        session.bypass = bypass;
                        module.sessions.set(&wallet, &session, state)?;
                    }
                }
            }

            module.emit_event(state, Event::BypassSet { wallet, bypass });

            Ok(())
        }
        CallMessage::SetExpiryOffset { new_offset } => {
            if !module.is_owner(context.sender(), state)? {
                return Err(SessionRegistryError::UnauthorizedOwner.into());
            }

            let old_offset = module.expiry_offset.get(state)?;

            module.expiry_offset.set(&new_offset, state)?;

            module.emit_event(
                state,
                Event::ExpiryOffsetUpdated {
                    old_offset,
                    new_offset,
                },
            );

            Ok(())
        }

        // --- Endpoints for direct session checks via transactions ---
        CallMessage::EnforceSessionActive { wallet } => {
            module.enforce_session_active(&wallet, state)
        }
        CallMessage::EnforceSessionPresent { wallet } => {
            module.enforce_session_present(&wallet, state)
        }
    }
}
