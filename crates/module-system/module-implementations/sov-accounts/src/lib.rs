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
    Context, CredentialId, DaSpec, GenesisState, Module, ModuleId, ModuleInfo, ModuleRestApi,
    SafeVec, Spec, StateMap, StateValue, TxState,
};

/// The maximum number of allowed credentials per account.
pub const ALLOWED_CREDENTIALS_PER_ACCOUNT: usize = 32;

/// An account on the rollup.
#[derive(
    borsh::BorshDeserialize,
    borsh::BorshSerialize,
    serde::Serialize,
    serde::Deserialize,
    Debug,
    PartialEq,
    Clone,
    Default,
)]
pub struct Account {
    /// The crednetials allowed to access the account.
    pub allowed_credentials: SafeVec<CredentialId, ALLOWED_CREDENTIALS_PER_ACCOUNT>,
}

/// A module responsible for managing accounts on the rollup.
#[derive(Clone, ModuleInfo, ModuleRestApi)]
#[cfg_attr(feature = "arbitrary", derive(Debug))]
pub struct Accounts<S: Spec> {
    /// The ID of the sov-accounts module.
    #[id]
    pub id: ModuleId,

    /// Mapping from a credential to its corresponding account.
    #[state]
    pub(crate) accounts: StateMap<S::Address, Account>,

    /// If this field is false, `CallMessage::InsertCredentialId` messages will be rejected.
    #[state]
    enable_custom_account_mappings: StateValue<bool>,
}

impl<S: Spec> Module for Accounts<S> {
    type Spec = S;

    type Config = AccountConfig<S>;

    type CallMessage = call::CallMessage;

    type Event = ();

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
    ) -> anyhow::Result<()> {
        match msg {
            call::CallMessage::InsertCredentialId(new_credential_id) => {
                Ok(self.insert_credential_id(new_credential_id, context, state)?)
            }
        }
    }
}

impl<S: Spec> Accounts<S> {
    /// Returns a reference to the accounts map.
    pub fn accounts(&self) -> &StateMap<S::Address, Account> {
        &self.accounts
    }
}
