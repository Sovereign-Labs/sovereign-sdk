#![deny(missing_docs)]
// Tombstone field `_accounts` makes the `ModuleInfo`-derived
// `_prefix__accounts` accessor double-underscored.
#![allow(non_snake_case)]
#![doc = include_str!("../README.md")]
mod call;
mod capabilities;
#[cfg(all(feature = "arbitrary", feature = "native"))]
mod fuzz;
mod genesis;
pub use genesis::*;
#[cfg(feature = "native")]
pub mod migrations;
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

/// Stored address for a legacy/custom credential-indexed account mapping.
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
    /// The mapped address.
    pub addr: S::Address,
}

/// Composite key for [`Accounts::account_owners`].
#[derive(
    borsh::BorshDeserialize, borsh::BorshSerialize, Debug, Clone, Copy, PartialEq, Eq, Hash,
)]
pub(crate) struct AccountOwnerKey<S: Spec> {
    address: S::Address,
    credential_id: CredentialId,
}

impl<S: Spec> AccountOwnerKey<S> {
    pub(crate) fn new(address: S::Address, credential_id: CredentialId) -> Self {
        Self {
            address,
            credential_id,
        }
    }
}

// `Display` / `FromStr` exist only to satisfy `StateMap`'s trait bound; on-chain
// keys are Borsh-serialized.
impl<S: Spec> std::fmt::Display for AccountOwnerKey<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.address, self.credential_id)
    }
}

impl<S: Spec> std::str::FromStr for AccountOwnerKey<S> {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (addr_str, cred_str) = s
            .rsplit_once('/')
            .ok_or_else(|| anyhow::anyhow!("invalid AccountOwnerKey: missing '/' separator"))?;
        Ok(Self {
            address: <S::Address as std::str::FromStr>::from_str(addr_str)
                .map_err(|e| anyhow::Error::from_boxed(e.into()))?,
            credential_id: cred_str.parse()?,
        })
    }
}

/// A module responsible for resolving credentials to addresses and recording
/// credential authorizations.
#[derive(Clone, ModuleInfo, ModuleRestApi)]
#[cfg_attr(feature = "arbitrary", derive(Debug))]
pub struct Accounts<S: Spec> {
    /// The ID of the sov-accounts module.
    #[id]
    pub id: ModuleId,

    /// Tombstone for the legacy `credential_id -> address` routing index.
    ///
    /// **Do not read or write outside [`crate::migrations`].** The field is
    /// retained only to preserve the `#[state]` field discriminant ordering
    /// derived by the `ModuleInfo` macro (this is the first state field, so
    /// removing it would shift the discriminants of every following field
    /// and corrupt their on-disk data). Existing entries are migrated to
    /// [`Self::account_owners`] by
    /// [`crate::migrations::apply_legacy_account_migration`] and the source
    /// rows are deleted. The leading underscore signals to readers that this
    /// field is intentionally unused.
    #[state]
    pub(crate) _accounts: StateMap<CredentialId, Account<S>>,

    /// If this field is false, configured genesis authorizations and
    /// `CallMessage::InsertCredentialId` messages will be rejected.
    #[state]
    enable_custom_account_mappings: StateValue<bool>,

    /// Authorization overrides. `Some(true)` means `credential_id` may sign as
    /// `address`; `Some(false)` explicitly revokes fallback authorization for
    /// the pair.
    #[state]
    pub(crate) account_owners: StateMap<AccountOwnerKey<S>, bool>,
}

impl<S: Spec> Module for Accounts<S> {
    type Spec = S;

    type Config = AccountConfig<S>;

    type CallMessage = call::CallMessage<S>;

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
                self.insert_credential_id(new_credential_id, context, state)
            }
            call::CallMessage::AddCredentialToAddress {
                address,
                credential,
            } => self.add_credential_to_address(address, credential, context, state),
            call::CallMessage::RemoveCredentialFromAddress {
                address,
                credential,
            } => self.remove_credential_from_address(address, credential, context, state),
            call::CallMessage::RotateCredentialOnAddress {
                address,
                old_credential,
                new_credential,
            } => self.rotate_credential_on_address(
                address,
                old_credential,
                new_credential,
                context,
                state,
            ),
        }
    }
}
