//! Traits used by all generators.

use std::collections::HashSet;
use std::hash::Hash;
use std::sync::Arc;

use sov_bank::TokenId;
use sov_modules_api::prelude::arbitrary::{self, Arbitrary};
use sov_modules_api::prelude::axum::async_trait;
use sov_modules_api::prelude::tokio::task::JoinSet;
use sov_modules_api::{DispatchCall, Module, Spec};

use super::{Percent, TagAction};
use crate::interface::{GeneratedMessage, MessageValidity};
use crate::state::State;
use crate::TokenInfo;

/// The state of the transaction generator, which is shared across all modules.
///
/// The generator state maintains a global state, but partitions it into "views" containing
/// information relevant to a single module. For example, to test the "bank" and sequencer
/// registry modules, the global state would looks something like this:
///
/// ```rust
/// use sov_bank::Coins;
/// use sov_modules_api::{CryptoSpec, Spec};
///
/// struct AccountState<S: Spec> {
///   pub balances: Vec<Coins>,
///   pub sequencing_bond: Option<u64>,
///   pub private_key: <S::CryptoSpec as CryptoSpec>::PrivateKey,
/// }
///
/// ```
///
/// In this example, the `Bank` account view would be constructed from the global
/// state by cloning the `balances` array, and the sequencer registry would
/// be constructed by cloning the `sequencing_bond`.
///
/// Splitting the GeneratorState for each module into its own view this way makes
/// it easy to reuse code.
pub trait GeneratorState<S: Spec> {
    /// The view of an account for a particular module
    type AccountView: Taggable<Tag: Into<Self::Tag>>;

    /// The `Tag` enum associated with this generator. Tags form a secondary index
    /// that can be used to quickly recover accounts matching certain criteria.
    type Tag: Hash + Eq;

    /// Creates a fresh copy of the appropriate view of the account with the given address.
    fn get_account(&self, address: &S::Address) -> Option<Self::AccountView>;

    /// Creates a fresh copy of the appropriate view of the account with the given tag.
    fn get_account_with_tag(&self, tag: Self::Tag) -> Option<Self::AccountView>;

    /// Picks an account at random from the generator state and returns a copy.
    /// If no account exists, generate a new one
    fn get_random_existing_account(
        &mut self,
        u: &mut arbitrary::Unstructured<'_>,
    ) -> arbitrary::Result<(S::Address, Self::AccountView)>;

    /// Picks an account at random from the generator state and returns a copy.
    fn get_random_existing_account_with_tag(
        &self,
        tag: Self::Tag,
        u: &mut arbitrary::Unstructured<'_>,
    ) -> arbitrary::Result<Option<(S::Address, Self::AccountView)>>;

    /// Returns true if the account has the given tag
    fn has_tag(&self, addr: &S::Address, tag: Self::Tag) -> bool;

    /// Gets the token with the given ID
    fn get_token(&self, id: &TokenId) -> Option<TokenInfo>;

    /// Gets the token with the given ID
    fn update_token(&mut self, id: TokenId, info: TokenInfo);

    /// Gets a random token
    fn get_random_token(
        &self,
        u: &mut arbitrary::Unstructured<'_>,
    ) -> arbitrary::Result<(TokenId, TokenInfo)>;

    /// Updates the given account to match the provided view.
    /// If the account for the provided address does not exist, it is created from the provided view.
    fn update_account(&mut self, address: &S::Address, view: Self::AccountView);

    /// Generates an empty account and returns it.
    fn generate_account(
        &mut self,
        u: &mut arbitrary::Unstructured<'_>,
    ) -> arbitrary::Result<(S::Address, Self::AccountView)>;

    /// Either picks or generates an account.
    fn get_or_generate(
        &mut self,
        generation_probability: Percent,
        u: &mut arbitrary::Unstructured<'_>,
    ) -> arbitrary::Result<(S::Address, Self::AccountView)> {
        if Percent::arbitrary(u)? < generation_probability {
            self.generate_account(u)
        } else {
            self.get_random_existing_account(u)
        }
    }
}

/// Allows items to be tagged
pub trait Taggable {
    /// The tag type
    type Tag;
    /// Takes the list of tags
    fn take_tags(&mut self) -> impl IntoIterator<Item = TagAction<Self::Tag>>;

    /// Adds the tag
    fn add_tag(&mut self, tag: Self::Tag);

    /// Removes the tag, if present
    fn remove_tag(&mut self, tag: Self::Tag);
}

/// Defines the interface of change entries that can be used to assert the state of a module after message generation.
#[async_trait]
pub trait ChangelogEntry: std::fmt::Debug + Send + Sync + 'static {
    /// A service which returns the current rollup state.
    type ClientConfig: 'static + Send + Sync;

    /// A discriminant that can be used to distinguish two [`ChangelogEntry`]s.
    type Discriminant: Eq + Hash;

    /// Assert that the rollup state matches the expected value.
    async fn assert_state(
        &self,
        rollup_state_accessor: Arc<Self::ClientConfig>,
    ) -> Result<(), anyhow::Error>;

    /// Transforms the [`ChangelogEntry`] into a discriminant that can be used to
    /// distinguish two [`ChangelogEntry`]s.
    fn as_discriminant(&self) -> Self::Discriminant;
}

/// Asserts all the [`ChangelogEntry`] logs against the existing state
pub async fn assert_logs_against_state<Log: ChangelogEntry>(
    logs: Vec<Log>,
    config: Arc<Log::ClientConfig>,
    num_threads: u8,
) -> anyhow::Result<()> {
    let mut seen_entries: HashSet<Log::Discriminant> = HashSet::new();
    let mut joinset = JoinSet::new();

    for log in logs.into_iter().rev() {
        if !seen_entries.insert(log.as_discriminant()) {
            continue;
        }

        let config_clone = config.clone();

        if joinset.len() >= num_threads as usize {
            joinset.join_next().await.unwrap()??;
        }

        joinset.spawn(async move { log.assert_state(config_clone).await });
    }

    let res = joinset.join_all().await;

    let err = res.iter().find(|res| res.is_err());

    if err.is_some() {
        anyhow::bail!("Some asserts failed. {:?}", err);
    }

    Ok(())
}

/// A standard interface for generating call messages and checking that they produce
/// the expected effects.
#[async_trait]
pub trait CallMessageGenerator<S: Spec> {
    /// The module for which the call message generator is being implemented
    type Module: Module<Spec = S>;

    /// The tag type used by this module, if applicable
    type Tag: std::fmt::Debug + Hash + Eq;

    /// The view of account state used by the module message generator
    type AccountView: Clone + std::fmt::Debug;

    /// The relevant post state from a generatd message.
    type ChangelogEntry: ChangelogEntry;

    /// Generate call messages needed to properly setup the generator.
    #[allow(clippy::type_complexity)]
    fn generate_setup_messages(
        &self,
        u: &mut arbitrary::Unstructured<'_>,
        generator_state: &mut impl GeneratorState<
            S,
            AccountView = Self::AccountView,
            Tag: From<Self::Tag>,
        >,
    ) -> arbitrary::Result<
        Vec<GeneratedMessage<S, <Self::Module as Module>::CallMessage, Self::ChangelogEntry>>,
    >;

    /// Generates a `CallMessage`, potentially valid or invalid, based on the provided parameters.
    fn generate_call_message(
        &self,
        u: &mut arbitrary::Unstructured<'_>,
        generator_state: &mut impl GeneratorState<
            S,
            AccountView = Self::AccountView,
            Tag: From<Self::Tag>,
        >,
        validity: MessageValidity,
    ) -> arbitrary::Result<
        GeneratedMessage<S, <Self::Module as Module>::CallMessage, Self::ChangelogEntry>,
    >;
}

/// A module that can be used to generate messages for a [`CallMessageGenerator`].
pub trait HarnessModule<S: Spec, RT: DispatchCall, Tag, CL: ChangelogEntry, BonusAcctData = ()>:
    Send + Sync
{
    /// Generates a list of setup messages for the module.
    #[allow(clippy::type_complexity)]
    fn generate_setup_messages(
        &self,
        u: &mut arbitrary::Unstructured<'_>,
        generator_state: &mut State<S, Tag, BonusAcctData>,
    ) -> arbitrary::Result<Vec<GeneratedMessage<S, <RT as DispatchCall>::Decodable, CL>>>;

    /// Generates a list of call messages for the module.
    #[allow(clippy::type_complexity)]
    fn generate_call_message(
        &self,
        u: &mut arbitrary::Unstructured<'_>,
        generator_state: &mut State<S, Tag, BonusAcctData>,
        validity: MessageValidity,
    ) -> arbitrary::Result<GeneratedMessage<S, <RT as DispatchCall>::Decodable, CL>>;
}
