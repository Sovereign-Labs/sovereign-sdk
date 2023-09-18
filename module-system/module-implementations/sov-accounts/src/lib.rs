#![deny(missing_docs)]
#![doc = include_str!("../README.md")]
mod hooks;

mod call;
mod genesis;
#[cfg(feature = "native")]
mod query;
#[cfg(feature = "native")]
pub use query::*;
#[cfg(test)]
mod tests;

pub use call::{CallMessage, UPDATE_ACCOUNT_MSG};
use sov_modules_api::{Context, Error, ModuleInfo, WorkingSet};

/// Initial configuration for sov-accounts module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountConfig<C: Context> {
    /// Public keys to initialize the rollup.
    pub pub_keys: Vec<C::PublicKey>,
}

impl<C: Context> FromIterator<C::PublicKey> for AccountConfig<C> {
    fn from_iter<T: IntoIterator<Item = C::PublicKey>>(iter: T) -> Self {
        Self {
            pub_keys: iter.into_iter().collect(),
        }
    }
}

/// An account on the rollup.
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq, Copy, Clone)]
pub struct Account<C: Context> {
    /// The address of the account.
    pub addr: C::Address,
    /// The current nonce value associated with the account.
    pub nonce: u64,
}

/// A module responsible for managing accounts on the rollup.
#[cfg_attr(feature = "native", derive(sov_modules_api::ModuleCallJsonSchema))]
#[derive(ModuleInfo, Clone)]
pub struct Accounts<C: Context> {
    /// The address of the sov-accounts module.
    #[address]
    pub address: C::Address,

    /// Mapping from an account address to a corresponding public key.
    #[state]
    pub(crate) public_keys: sov_modules_api::StateMap<C::Address, C::PublicKey>,

    /// Mapping from a public key to a corresponding account.
    #[state]
    pub(crate) accounts: sov_modules_api::StateMap<C::PublicKey, Account<C>>,
}

impl<C: Context> sov_modules_api::Module for Accounts<C> {
    type Context = C;

    type Config = AccountConfig<C>;

    type CallMessage = call::CallMessage<C>;

    fn genesis(&self, config: &Self::Config, working_set: &mut WorkingSet<C>) -> Result<(), Error> {
        Ok(self.init_module(config, working_set)?)
    }

    fn call(
        &self,
        msg: Self::CallMessage,
        context: &Self::Context,
        working_set: &mut WorkingSet<C>,
    ) -> Result<sov_modules_api::CallResponse, Error> {
        match msg {
            call::CallMessage::UpdatePublicKey(new_pub_key, sig) => {
                Ok(self.update_public_key(new_pub_key, sig, context, working_set)?)
            }
        }
    }
}

#[cfg(feature = "arbitrary")]
impl<'a, C> arbitrary::Arbitrary<'a> for Account<C>
where
    C: Context,
    C::Address: arbitrary::Arbitrary<'a>,
{
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let addr = u.arbitrary()?;
        let nonce = u.arbitrary()?;
        Ok(Self { addr, nonce })
    }
}

#[cfg(feature = "arbitrary")]
impl<'a, C> arbitrary::Arbitrary<'a> for AccountConfig<C>
where
    C: Context,
    C::PublicKey: arbitrary::Arbitrary<'a>,
{
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        // TODO we might want a dedicated struct that will generate the private key counterpart so
        // payloads can be signed and verified
        Ok(Self {
            pub_keys: u.arbitrary_iter()?.collect::<Result<_, _>>()?,
        })
    }
}

#[cfg(feature = "arbitrary")]
impl<'a, C> Accounts<C>
where
    C: Context,
    C::Address: arbitrary::Arbitrary<'a>,
    C::PublicKey: arbitrary::Arbitrary<'a>,
{
    /// Creates an arbitrary set of accounts and stores it under `working_set`.
    pub fn arbitrary_workset(
        u: &mut arbitrary::Unstructured<'a>,
        working_set: &mut WorkingSet<C>,
    ) -> arbitrary::Result<Self> {
        use sov_modules_api::Module;

        let config: AccountConfig<C> = u.arbitrary()?;
        let accounts = Accounts::default();

        accounts
            .genesis(&config, working_set)
            .map_err(|_| arbitrary::Error::IncorrectFormat)?;

        Ok(accounts)
    }
}
