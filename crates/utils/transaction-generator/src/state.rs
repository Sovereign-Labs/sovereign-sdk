//! Provides utilities to track the state of the testing harness.
use std::collections::HashMap;
use std::hash::Hash;

use indexmap::{IndexMap, IndexSet};
use sov_bank::{config_gas_token_id, Amount, Coins, TokenId};
use sov_modules_api::{CryptoSpec, PrivateKey, PublicKey, Spec};

/// A view into `AccountState` containing some subset of its data. Identical to `AccountState` except that all fields
/// are wrapped in an `Option` so that irrelevant fields can be ignored.
#[derive(Clone, Debug)]
pub struct AccountState<S: Spec, Data = ()> {
    /// The account's balances
    pub balances: Vec<Coins>,
    /// The set of known tokens which this account is allowed to mint
    pub can_mint: IndexSet<TokenId>,
    /// The bond amount that this account has locked in the sequencer registry, if applicable
    pub sequencing_bond: Option<u64>,
    /// The private key for this account
    pub private_key: <S::CryptoSpec as CryptoSpec>::PrivateKey,
    /// Any additional state tracked by external modules
    pub additional_info: Data,
}

impl<S: Spec, T: Default> AccountState<S, T> {
    /// Create an empty account with the given private key
    pub fn with_private_key(private_key: <S::CryptoSpec as CryptoSpec>::PrivateKey) -> Self {
        Self {
            balances: Vec::new(),
            can_mint: Default::default(),
            sequencing_bond: None,
            private_key,
            additional_info: Default::default(),
        }
    }
}

/// Allows a state view to update the global state.
pub trait ApplyToState<S: Spec, T = ()> {
    /// Applies any changes to a view onto the global account state
    fn apply_to(self, account: &mut AccountState<S, T>);
}

/// The token info needed for message generation
#[derive(Debug, Clone)]
pub struct TokenInfo {
    /// The current supply of the token
    pub total_supply: Amount,
}

/// A struct that tracks the global state used by the transaction generator. Tracks accounts by address
/// and maintains a secondary index using the `tags` provided by the module.
#[derive(Clone)]
pub struct State<S: Spec, Tag, T = ()> {
    pub(crate) accounts: IndexMap<S::Address, AccountState<S, T>>,
    pub(crate) tags: HashMap<Tag, IndexSet<S::Address>>,
    pub(crate) tokens: IndexMap<TokenId, TokenInfo>,
}

impl<S: Spec, Tag, T> Default for State<S, Tag, T> {
    fn default() -> Self {
        Self {
            accounts: Default::default(),
            tokens: Default::default(),
            tags: Default::default(),
        }
    }
}

impl<S: Spec, Tag: Eq + Hash, T> State<S, Tag, T> {
    /// Returns a reference to the state accounts
    pub fn accounts(&self) -> &IndexMap<S::Address, AccountState<S, T>> {
        &self.accounts
    }

    /// Returns a reference to the state tags
    pub fn tags(&self) -> &HashMap<Tag, IndexSet<S::Address>> {
        &self.tags
    }

    /// Create an empty [`State`].
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a new state containing the provided account. Tags the account with the provided tags
    /// and intitializes the token_supply tracker for any relevant tokens. This method assumes that
    /// the account holder is the *only* holder of any tokens. If that assumption is violated, message
    /// generation may fail.
    pub fn with_account_and_tags(account: AccountState<S, T>, tags: Vec<Tag>) -> Self {
        let mut output = Self::default();
        let address: <S as Spec>::Address = account.private_key.pub_key().credential_id().into();
        for tag in tags {
            output.tags.entry(tag).or_default().insert(address.clone());
        }
        for balance in account.balances.iter() {
            let duplicate = output.tokens.insert(
                balance.token_id,
                TokenInfo {
                    total_supply: balance.amount,
                },
            );
            if balance.token_id == config_gas_token_id() {
                tracing::warn!("Using gas token for bank messsage generation. This can cause unexpected behavior because of gas payments!");
            }
            assert!(
                duplicate.is_none(),
                "Duplicate balances in initial account state"
            );
        }
        output.accounts.insert(address, account);

        output
    }

    /// Insert an outside account into state.
    pub fn insert_account(&mut self, account: AccountState<S, T>) {
        let address: <S as Spec>::Address = account.private_key.pub_key().credential_id().into();
        self.accounts.insert(address, account);
    }
}
