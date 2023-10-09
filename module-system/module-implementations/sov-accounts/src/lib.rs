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
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(bound = "C::PublicKey: serde::Serialize + serde::de::DeserializeOwned")]
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
#[cfg_attr(feature = "arbitrary", derive(Debug))]
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
mod arbitrary_impls {
    use arbitrary::{Arbitrary, Unstructured};
    use proptest::arbitrary::any;
    use proptest::prelude::RngCore;
    use proptest::strategy::{BoxedStrategy, Strategy};
    use sov_modules_api::{Module, PrivateKey, StateMap};
    use sov_state::Prefix;

    use super::*;

    use std::sync::{Arc, Mutex};

    impl<'a, C> Arbitrary<'a> for Account<C>
    where
        C: Context,
        C::Address: Arbitrary<'a>,
    {
        fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
            let addr = u.arbitrary()?;
            let nonce = u.arbitrary()?;
            Ok(Self { addr, nonce })
        }
    }

    impl<C> proptest::arbitrary::Arbitrary for Account<C>
    where
        C: Context,
        C::Address: proptest::arbitrary::Arbitrary,
    {
        type Parameters = ();
        type Strategy = BoxedStrategy<Self>;

        fn arbitrary_with(_args: Self::Parameters) -> Self::Strategy {
            (any::<C::Address>(), any::<u64>())
                .prop_map(|(addr, nonce)| Account { addr, nonce })
                .boxed()
        }
    }

    impl<'a, C> Arbitrary<'a> for AccountConfig<C>
    where
        C: Context,
        C::PublicKey: Arbitrary<'a>,
    {
        fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
            // TODO we might want a dedicated struct that will generate the private key counterpart so
            // payloads can be signed and verified
            Ok(Self {
                pub_keys: u.arbitrary_iter()?.collect::<Result<_, _>>()?,
            })
        }
    }

    impl<C> proptest::arbitrary::Arbitrary for AccountConfig<C>
    where
        C: Context,
        C::PrivateKey: proptest::arbitrary::Arbitrary,
    {
        type Parameters = ();
        type Strategy = BoxedStrategy<Self>;

        fn arbitrary_with(_args: Self::Parameters) -> Self::Strategy {
            any::<Vec<C::PrivateKey>>()
                .prop_map(|keys| AccountConfig {
                    pub_keys: keys.into_iter().map(|k| k.pub_key()).collect(),
                })
                .boxed()
        }
    }

    impl<'a, C> Accounts<C>
    where
        C: Context,
        C::Address: Arbitrary<'a>,
        C::PublicKey: Arbitrary<'a>,
    {
        /// Creates an arbitrary set of accounts and stores it under `working_set`.
        pub fn arbitrary_workset(
            u: &mut Unstructured<'a>,
            working_set: &mut WorkingSet<C>,
        ) -> arbitrary::Result<Self> {
            let config: AccountConfig<C> = u.arbitrary()?;
            let accounts = Accounts::default();

            accounts
                .genesis(&config, working_set)
                .map_err(|_| arbitrary::Error::IncorrectFormat)?;

            Ok(accounts)
        }
    }

    impl<C> Accounts<C>
    where
        C: Context,
        C::Address: proptest::arbitrary::Arbitrary,
        C::PrivateKey: proptest::arbitrary::Arbitrary,
    {
        /// Creates an arbitrary set of accounts and stores it under `working_set`.
        ///
        /// We take the `WorkingSet` as `Arc<Mutex<_>>` so the strategy can freely implement
        /// parallelism while preserving interior mutability safety. The return is a reference to a
        /// promise of a strategy; hence, the `WorkingSet` lifetime must be locked to its lifetime.
        pub fn arbitrary_proptest_workset(
            working_set: Arc<Mutex<WorkingSet<C>>>,
        ) -> impl Strategy<Value = Self> {
            any::<(C::Address, AccountConfig<C>, Prefix, Prefix)>()
                .prop_perturb(
                    move |(address, config, prefix_keys, prefix_accounts), mut rng| {
                        let mut working_set_lock =
                            working_set.lock().expect("working set lock is poisoned");
                        let working_set = &mut *working_set_lock;
                        let public_keys = StateMap::new(prefix_keys);
                        let accounts = StateMap::new(prefix_accounts);

                        for public in config.pub_keys {
                            let mut address = [0u8; 32];
                            rng.fill_bytes(&mut address);

                            let addr = C::Address::from(address);
                            let nonce = rng.next_u64();
                            let account: Account<C> = Account { addr, nonce };

                            public_keys.set(&account.addr, &public, working_set);
                            accounts.set(&public, &account, working_set);
                        }

                        Accounts {
                            address,
                            public_keys,
                            accounts,
                        }
                    },
                )
                .boxed()
        }
    }
}
