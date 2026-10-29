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
use sov_modules_api::macros::serialize;
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

/// Events emitted by the [`Accounts`] module.
#[derive(Debug, PartialEq, Eq, Clone, schemars::JsonSchema)]
#[serialize(Borsh, Serde)]
#[serde(bound = "S: Spec", rename_all = "snake_case")]
#[schemars(bound = "S::Address: ::schemars::JsonSchema", rename = "Event")]
pub enum Event<S: Spec> {
    /// Emitted by [`CallMessage::CreateSyntheticAddress`] when a new
    /// synthetic address — an address with no naturally-corresponding
    /// private key — is created and the caller's credential is
    /// auto-authorized for it. Consumers should treat this event as the
    /// canonical source of the derived address: because the derivation
    /// is bound to the visible slot hash, it can only be reproduced
    /// after the creation tx is finalized.
    SyntheticAddressCreated {
        /// The newly created address.
        address: S::Address,
        /// The address that submitted the creation transaction.
        creator: S::Address,
        /// The credential authorized to control the new address.
        credential: CredentialId,
    },
    /// Emitted by [`CallMessage::InsertCredentialId`] and
    /// [`CallMessage::AddCredentialToAddress`] when a new credential is
    /// authorized for an address.
    CredentialAdded {
        /// The address whose credential set was extended.
        address: S::Address,
        /// The newly authorized credential.
        credential: CredentialId,
        /// The address that submitted the authorizing transaction. Today
        /// equals `address` (handlers require `context.sender() == address`);
        /// recorded separately for audit.
        authorizer_address: S::Address,
    },
    /// Emitted by [`CallMessage::RemoveCredentialFromAddress`] when a
    /// credential is revoked from an address.
    CredentialRemoved {
        /// The address whose credential set was reduced.
        address: S::Address,
        /// The revoked credential.
        credential: CredentialId,
        /// The address that submitted the revoking transaction. Today
        /// equals `address` (handlers require `context.sender() == address`);
        /// recorded separately for audit.
        authorizer_address: S::Address,
    },
    /// Emitted by [`CallMessage::RotateCredentialOnAddress`] when a
    /// credential is atomically swapped for another on an address.
    CredentialRotated {
        /// The address whose credential set was rotated.
        address: S::Address,
        /// The revoked credential.
        old_credential: CredentialId,
        /// The newly authorized credential.
        new_credential: CredentialId,
        /// The address that submitted the rotating transaction. Today
        /// equals `address` (handlers require `context.sender() == address`);
        /// recorded separately for audit.
        authorizer_address: S::Address,
    },
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
pub struct Accounts<S: Spec> {
    /// The ID of the sov-accounts module.
    #[id]
    pub id: ModuleId,

    /// Chain-state module, used to read the visible DA slot hash when
    /// deriving synthetic addresses.
    #[module]
    pub(crate) chain_state: sov_chain_state::ChainState<S>,

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
    ///
    /// **Invariant:** entries are written *only* by [`Self::authorize_credential`]
    /// (writing `true`) and by [`crate::call::Accounts::revoke_credential`]
    /// (writing `false`). The absence of an entry — `None` — therefore reliably
    /// means "no call has ever touched this `(address, credential)` pair", which
    /// is what [`crate::call::Accounts::create_synthetic_address`] relies on to
    /// keep replay-after-revoke a no-op. Adding a new writer that does not
    /// preserve this convention breaks that guarantee.
    #[state]
    pub(crate) account_owners: StateMap<AccountOwnerKey<S>, bool>,
}

impl<S: Spec> Module for Accounts<S> {
    type Spec = S;

    type Config = AccountConfig<S>;

    type CallMessage = call::CallMessage<S>;

    type Event = Event<S>;

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
            call::CallMessage::CreateSyntheticAddress { salt } => {
                self.create_synthetic_address(salt, context, state)
            }
        }
    }
}
