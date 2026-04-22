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

/// Composite key for [`Accounts::account_owners`]. A present entry
/// `(address, credential_id)` means `credential_id` is authorized to sign
/// transactions that execute as `address`.
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
            address: addr_str
                .parse()
                .map_err(|e| anyhow::anyhow!("invalid address in AccountOwnerKey: {e:?}"))?,
            credential_id: cred_str
                .parse()
                .map_err(|e| anyhow::anyhow!("invalid credential_id in AccountOwnerKey: {e:?}"))?,
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

    /// Legacy/custom `credential_id -> address` routing index. New
    /// authorization writes use [`Self::account_owners`] instead.
    #[state]
    pub(crate) accounts: StateMap<CredentialId, Account<S>>,

    /// If this field is false, `CallMessage::InsertCredentialId` messages will be rejected.
    #[state]
    enable_custom_account_mappings: StateValue<bool>,

    /// Authorization set: a present entry means `credential_id` may sign as
    /// `address`.
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
