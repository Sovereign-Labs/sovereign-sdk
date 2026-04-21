#![deny(missing_docs)]
#![doc = include_str!("../README.md")]
mod call;
mod capabilities;
#[cfg(all(feature = "arbitrary", feature = "native"))]
mod fuzz;
mod genesis;
pub use genesis::*;
#[cfg(feature = "native")]
mod query;
#[cfg(feature = "native")]
pub use query::*;
#[cfg(test)]
mod tests;
pub use call::CallMessage;
use sov_modules_api::{
    Context, CredentialId, DaSpec, GenesisState, Module, ModuleId, ModuleInfo, ModuleRestApi, Spec,
    StateMap, StateValue, TxState,
};

/// An account on the rollup.
#[derive(
    borsh::BorshDeserialize,
    borsh::BorshSerialize,
    serde::Serialize,
    serde::Deserialize,
    Debug,
    PartialEq,
    Copy,
    Clone,
)]
pub struct Account<S: Spec> {
    /// The address of the account.
    pub addr: S::Address,
}

/// Composite key used by [`Accounts::account_owners`]. A present entry
/// `(address, credential_id)` means `credential_id` is authorized to spend as
/// `address`. Many credentials may authorize one address, and one credential may
/// be authorized for many addresses.
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct AccountOwnerKey<S: Spec> {
    pub(crate) address: S::Address,
    pub(crate) credential_id: CredentialId,
}

impl<S: Spec> AccountOwnerKey<S> {
    pub(crate) fn new(address: S::Address, credential_id: CredentialId) -> Self {
        Self {
            address,
            credential_id,
        }
    }
}

impl<S: Spec> std::fmt::Display for AccountOwnerKey<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.address, self.credential_id)
    }
}

impl<S: Spec> std::str::FromStr for AccountOwnerKey<S> {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (addr_str, cred_str) = s
            .split_once('/')
            .ok_or_else(|| anyhow::anyhow!("invalid AccountOwnerKey: missing '/' separator"))?;
        Ok(Self {
            address: addr_str
                .parse()
                .map_err(|e| anyhow::anyhow!("invalid address in AccountOwnerKey: {e:?}"))?,
            credential_id: cred_str
                .parse()
                .map_err(|e| anyhow::anyhow!("invalid credential_id in AccountOwnerKey: {e}"))?,
        })
    }
}

/// A module responsible for managing accounts on the rollup.
#[derive(Clone, ModuleInfo, ModuleRestApi)]
#[cfg_attr(feature = "arbitrary", derive(Debug))]
pub struct Accounts<S: Spec> {
    /// The ID of the sov-accounts module.
    #[id]
    pub id: ModuleId,

    /// Legacy `credential_id -> address` routing override. **Read-only post-PR-1:
    /// no code path in this module writes to this map.** Genesis, auto-register,
    /// and `InsertCredentialId` now write exclusively to [`Self::account_owners`].
    /// This field is retained to read pre-upgrade on-chain entries that were
    /// written before the freeze; `resolve_sender_address` falls back to it when
    /// the credential has no matching entry in `account_owners`. A follow-up PR
    /// will drain these entries into `account_owners` and drop this field.
    #[state]
    pub(crate) accounts: StateMap<CredentialId, Account<S>>,

    /// If this field is false, `CallMessage::InsertCredentialId` messages will be rejected.
    #[state]
    enable_custom_account_mappings: StateValue<bool>,

    /// Many-to-many authorization relation. A present entry
    /// `(address, credential_id)` means `credential_id` is authorized to sign
    /// transactions that execute as `address`. Unlike [`Self::accounts`], the
    /// same credential can own multiple addresses here. Authoritative source of
    /// authorization state for all post-PR-1 writes; a future transaction
    /// envelope (with a signed target address) will consult this map to decide
    /// whether the credential is allowed on the target's behalf.
    ///
    /// The value is a `bool` sentinel rather than `()` because the state
    /// backend does not preserve zero-byte values across commits — a `()`
    /// value survives in-session but the key is dropped on persist.
    #[state]
    pub(crate) account_owners: StateMap<AccountOwnerKey<S>, bool>,
}

impl<S: Spec> Module for Accounts<S> {
    type Spec = S;

    type Config = AccountConfig<S>;

    type CallMessage = call::CallMessage;

    type Event = ();

    type Error = anyhow::Error;

    fn genesis(
        &mut self,
        _genesis_rollup_header: &<<S as Spec>::Da as DaSpec>::BlockHeader,
        config: &Self::Config,
        state: &mut impl GenesisState<S>,
    ) -> anyhow::Result<()> {
        self.init_module(config, state)
    }

    fn call(
        &mut self,
        msg: Self::CallMessage,
        context: &Context<S>,
        state: &mut impl TxState<S>,
    ) -> Result<(), Self::Error> {
        match msg {
            call::CallMessage::InsertCredentialId(new_credential_id) => {
                Ok(self.insert_credential_id(new_credential_id, context, state)?)
            }
        }
    }
}
