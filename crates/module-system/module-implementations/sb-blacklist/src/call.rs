//! Call messages and execution entrypoint for the `Blacklist` module.

use schemars::JsonSchema;
use sov_modules_api::macros::UniversalWallet;
use sov_modules_api::{Context, Spec, TxState, EventEmitter};
use sov_modules_api::macros::serialize;

use crate::{Event, Blacklist, BlacklistError};

/// Transaction-level messages supported by the `Blacklist`.
///
/// Access control is enforced in [`execute`]:
/// - `SetManager`: owner-only
/// - `SetEnforcementEnabled`: owner-only
/// - `SetBlacklistSigner`: manager-only
/// - `SetBlacklisted` / `SetBlacklistedBatch`: blacklist-signer-only
#[derive(Debug, Clone, PartialEq, Eq, JsonSchema, UniversalWallet)]
#[serialize(Borsh, Serde)]
#[serde(rename_all = "snake_case")]
#[schemars(bound = "S: Spec", rename = "CallMessage")]
pub enum CallMessage<S: Spec> {
    /// Update the manager address.
    SetManager {
        new_manager: S::Address,
    },

    /// Enable or disable global blacklist enforcement.
    SetEnforcementEnabled {
        enabled: bool,
    },

    /// Grant or revoke blacklist-signer privileges for an address.
    SetBlacklistSigner {
        signer: S::Address,
        allowed: bool,
    },

    /// Set or clear the blacklist status for a single wallet.
    ///
    /// `blacklisted == true` adds the wallet to the blacklist.
    /// `blacklisted == false` removes it.
    SetBlacklisted {
        wallet: S::Address,
        blacklisted: bool,
    },

    /// Set or clear the blacklist status for a batch of wallets.
    SetBlacklistedBatch {
        wallets: Vec<S::Address>,
        blacklisted: Vec<bool>,
    },

    /// Assert that a wallet is NOT blacklisted.
    ///
    /// Fails if the wallet is in the blacklist
    EnforceNotBlacklisted {
        wallet: S::Address,
    },
}

/// Route a CallMessage to the corresponding `Blacklist` logic.
pub fn execute<S: Spec>(
    module: &mut Blacklist<S>,
    msg: CallMessage<S>,
    context: &Context<S>,
    state: &mut impl TxState<S>,
) -> anyhow::Result<()> {
    match msg {
        CallMessage::SetManager { new_manager } => {
            if !module.is_owner(context.sender(), state)? {
                return Err(BlacklistError::UnauthorizedOwner.into());
            }

            let old_manager = module.manager.get(state)?;

            module.manager.set(&new_manager, state)?;

            module.emit_event(
                state,
                Event::ManagerSet { old_manager, new_manager },
            );

            Ok(())
        }
        CallMessage::SetEnforcementEnabled { enabled } => {
            if !module.is_owner(context.sender(), state)? {
                return Err(BlacklistError::UnauthorizedOwner.into());
            }

            module.enforcement_enabled.set(&enabled, state)?;

            module.emit_event(
                state,
                Event::EnforcementEnabledSet { enabled },
            );

            Ok(())
        }
        CallMessage::SetBlacklistSigner { signer, allowed } => {
            if !module.is_manager(context.sender(), state)? {
                return Err(BlacklistError::UnauthorizedManager.into());
            }

            if allowed {
                module.blacklist_signers.set(&signer, &true, state)?;
            } else {
                module.blacklist_signers.remove(&signer, state)?;
            }

            module.emit_event(
                state,
                Event::BlacklistSignerSet { signer, allowed },
            );

            Ok(())
        }
        CallMessage::SetBlacklisted { wallet, blacklisted } => {
            if !module.is_blacklist_signer(context.sender(), state)? {
                return Err(BlacklistError::UnauthorizedBlacklistSigner.into());
            }

            module.write_blacklist(&wallet, blacklisted, state)?;

            Ok(())
        }
        CallMessage::SetBlacklistedBatch { wallets, blacklisted } => {
            if !module.is_blacklist_signer(context.sender(), state)? {
                return Err(BlacklistError::UnauthorizedBlacklistSigner.into());
            }

            if wallets.len() != blacklisted.len() {
                return Err(BlacklistError::InvalidBatchLengths.into());
            }

            for (wallet, is_blacklisted) in wallets.iter().zip(blacklisted.iter().copied()) {
                module.write_blacklist(wallet, is_blacklisted, state)?;
            }

            Ok(())
        }
        CallMessage::EnforceNotBlacklisted { wallet } => {
            module.enforce_not_blacklisted(&wallet, state)
        }
    }
}
