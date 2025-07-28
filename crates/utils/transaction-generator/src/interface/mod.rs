//! Defines the traits and types that form the interface for callmessage generation
mod rng;
use std::fmt::Debug;
use std::hash::Hash;
use std::marker::PhantomData;

pub mod traits;
use derive_more::derive::AsRef;
use derive_more::{Add, Mul};
pub use rng::*;
use serde::{Deserialize, Serialize};
use sov_bank::TokenId;
use sov_modules_api::prelude::arbitrary::{self, Arbitrary};
use sov_modules_api::{CryptoSpec, PrivateKey, PublicKey, Spec};
pub use traits::*;

use super::state::ApplyToState;
use crate::state::{AccountState, State, TokenInfo};

/// Whether a generated message should be valid or invalid.
#[derive(strum::EnumIs, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageValidity {
    #[allow(missing_docs)]
    Valid,
    #[allow(missing_docs)]
    Invalid,
}

impl MessageValidity {
    /// Make a distribution of message validity with the provided percentage of valid messages
    pub fn as_distribution(percentage_valid_messages: Percent) -> Distribution<MessageValidity> {
        Distribution::with_values(vec![
            (percentage_valid_messages.0 as u64, MessageValidity::Valid),
            (
                (100 - percentage_valid_messages.0) as u64,
                MessageValidity::Invalid,
            ),
        ])
    }
}

/// The outcome expected from the generated message
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageOutcome<E> {
    /// The message should execute successfully
    Successful {
        /// The changes to apply to the state
        changes: Vec<E>,
    },
    /// The message should have reverted
    Reverted,
}

impl<E> MessageOutcome<E> {
    /// Returns true if the outcome is expected to be successful
    pub fn is_successful(&self) -> bool {
        matches!(self, MessageOutcome::Successful { .. })
    }

    /// Returns true if the outcome is expected to be reverted
    pub fn is_reverted(&self) -> bool {
        matches!(self, MessageOutcome::Reverted)
    }

    /// Maps one type of message outcome to another
    pub fn map<F>(self, map_fn: impl FnMut(E) -> F) -> MessageOutcome<F> {
        match self {
            MessageOutcome::Reverted => MessageOutcome::Reverted,
            MessageOutcome::Successful { changes } => MessageOutcome::Successful {
                changes: changes.into_iter().map(map_fn).collect(),
            },
        }
    }

    /// Takes the changes from the message outcome. If the expected outcome is reverted,
    /// returns an empty array
    pub fn unwrap_changes(self) -> Vec<E> {
        match self {
            MessageOutcome::Reverted => vec![],
            MessageOutcome::Successful { changes } => changes,
        }
    }
}

/// A generated message for a particular module.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratedMessage<S: Spec, M, E: ChangelogEntry> {
    /// The generated call message
    pub message: M,
    /// The private key that should sign the message
    pub sender: <S::CryptoSpec as CryptoSpec>::PrivateKey,
    /// A summary of the changes that the transaction will make.
    pub outcome: MessageOutcome<E>,
}

impl<S: Spec, M, E: ChangelogEntry> GeneratedMessage<S, M, E> {
    /// Create a new [`GeneratedMessage`] from its components
    pub fn new(
        message: M,
        sender: <S::CryptoSpec as CryptoSpec>::PrivateKey,
        outcome: MessageOutcome<E>,
    ) -> Self {
        Self {
            message,
            sender,
            outcome,
        }
    }
}

/// A percentage, expressed as a whole number. Typically used to express the probability that some branch of the
/// generator will be taken. To decide whether to take a particular branch, use the pattern...
///
/// ```rust
///# use sov_modules_api::prelude::arbitrary;
///  use arbitrary::Arbitrary;
///# use sov_transaction_generator::interface::Percent;
///
///
/// fn should_take_branch<'a>(likelihood: Percent, u: &mut arbitrary::Unstructured<'a>) -> bool {
///     let random = Percent::arbitrary(u).unwrap();
///     if random < likelihood {
///         true
///     } else {
///         false
///     }
/// }
/// ```
#[derive(
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Clone,
    Copy,
    Hash,
    Debug,
    Add,
    Mul,
    AsRef,
    Serialize,
    Deserialize,
)]
pub struct Percent(u8);

impl std::ops::Sub for Percent {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        Self(self.0 - rhs.0)
    }
}

impl Percent {
    /// Returns 100 percent
    pub const fn one_hundred() -> Self {
        Self(100)
    }

    /// Returns fifty percent
    pub const fn fifty() -> Self {
        Self(50)
    }

    /// Returns zero
    pub const fn zero() -> Self {
        Self(0)
    }
}

impl std::iter::Sum for Percent {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        let mut sum = 0;
        for item in iter {
            sum += item.0;
        }
        Percent(sum)
    }
}

impl<'a> Arbitrary<'a> for Percent {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        Ok(Self(u.int_in_range(0..=99)?))
    }
}

impl PartialEq<u8> for Percent {
    fn eq(&self, rhs: &u8) -> bool {
        self.0 == *rhs
    }
}

/// A distribution of probabilities, expressed as relative weights. Optionally, the
/// distribution may have associated values. This makes it easier to keep the weights
/// and values in sync
///
/// # Examples
///
/// ```rust
///# use sov_transaction_generator::interface::Distribution;
///
/// Distribution::new(vec![1, 1, 1]); // Selects each of the three possibilities one third of the time
/// Distribution::new(vec![3, 1, 6]); // Assigns 30%, 10%, and 60% probabilities to each of three possibilities
/// Distribution::new(vec![3, 1, 6]); // Assigns 30%, 10%, and 60% probabilities to each of three possibilities
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Distribution<T = ()> {
    weights_and_values: Vec<(Percent, T)>,
}

impl<T> Distribution<T> {
    /// Returns the inner array of weights and values
    pub fn inner(&self) -> &Vec<(Percent, T)> {
        &self.weights_and_values
    }

    /// Creates a new distribution with the given weights
    pub fn with_values(weights_and_values: Vec<(u64, T)>) -> Self {
        let sum: u128 = weights_and_values.iter().map(|v| v.0 as u128).sum();
        assert_ne!(
            sum, 0,
            "Impossible to build a distribution with null weights"
        );

        let weights_and_values = weights_and_values
            .into_iter()
            .map(|(weight, value)| {
                (
                    Percent(
                        (((weight * 100) as u128) / sum)
                            .try_into()
                            .expect("Sum should always be above parts (when parts >= 0)"),
                    ),
                    value,
                )
            })
            .collect();

        Self { weights_and_values }
    }

    /// Creates a new distribution with the given weights
    pub fn with_equiprobable_values(values: Vec<T>) -> Self {
        let weights_and_values = values.into_iter().map(|value| (1, value)).collect();
        Self::with_values(weights_and_values)
    }

    /// Pick from the distribution at random. Return a usize in range `0..len(self.weights_and_values)``
    pub fn select_idx(&self, u: &mut arbitrary::Unstructured<'_>) -> arbitrary::Result<usize> {
        let mut target: u8 = u.int_in_range(1..=100)?;

        for (idx, (percent, _)) in self.weights_and_values.iter().enumerate() {
            if target <= percent.0 {
                return Ok(idx);
            } else {
                target -= percent.0;
            }
        }

        Ok(self.weights_and_values.len().saturating_sub(1))
    }

    /// Pick from the values at random.
    pub fn select_value(&self, u: &mut arbitrary::Unstructured<'_>) -> arbitrary::Result<&T> {
        Ok(&self.weights_and_values[self.select_idx(u)?].1)
    }

    /// Maps the values inside the distribution using the mapping function
    pub fn map_values<U>(self, map_fn: &mut impl (FnMut(T) -> U)) -> Distribution<U> {
        Distribution {
            weights_and_values: self
                .weights_and_values
                .into_iter()
                .map(|(p, t)| (p, map_fn(t)))
                .collect(),
        }
    }
}

impl Distribution {
    /// Creates a new distribution with the given weights
    pub fn new(weights: Vec<u64>) -> Self {
        let weights_and_values = weights.into_iter().map(|weight| (weight, ())).collect();
        Self::with_values(weights_and_values)
    }

    /// Take all branches with equal probability
    pub fn equiprobable(n: usize) -> Self {
        Self::new(vec![1; n])
    }
}

/// Converts a more general `GeneratorState` implementation to a more specific one - for example,
/// the `GeneratorState` for a whole runtime to the state for a particular module
pub struct GeneratorStateMapper<'a, S: Spec, Acct, Tag, T = ()>(
    &'a mut State<S, Tag, T>,
    PhantomData<Acct>,
);

impl<'a, S: Spec, Acct, Tag, T> GeneratorStateMapper<'a, S, Acct, Tag, T> {
    /// Create  a new [`GeneratorStateMapper`]
    pub fn new(state: &'a mut State<S, Tag, T>) -> Self {
        Self(state, PhantomData)
    }
}

impl<Acct, Tag, S: Spec, T: Default + Clone + 'static> GeneratorState<S>
    for GeneratorStateMapper<'_, S, Acct, Tag, T>
where
    Acct: Debug
        + Clone
        + for<'b> From<&'b AccountState<S, T>>
        + ApplyToState<S, T>
        + Taggable<Tag: Into<Tag>>,

    Tag: Eq + Hash + Debug + Clone,
{
    type AccountView = Acct;

    type Tag = Tag;

    fn get_account(&self, address: &S::Address) -> Option<Self::AccountView> {
        self.0.accounts.get(address).map(|acct| acct.into())
    }

    fn get_account_with_tag(&self, tag: Self::Tag) -> Option<Self::AccountView> {
        let address = self.0.tags.get(&tag).and_then(|set| set.first());

        address.and_then(|address: &S::Address| self.get_account(address))
    }

    fn get_random_existing_account(
        &mut self,
        u: &mut arbitrary::Unstructured<'_>,
    ) -> arbitrary::Result<(S::Address, Self::AccountView)> {
        if self.0.accounts.is_empty() {
            self.generate_account(u)?;
        }

        let (address, account) = self.0.accounts.random_entry(u)?;
        Ok((address.clone(), account.into()))
    }

    fn get_random_existing_account_with_tag(
        &self,
        tag: Self::Tag,
        u: &mut arbitrary::Unstructured<'_>,
    ) -> arbitrary::Result<Option<(S::Address, Self::AccountView)>> {
        if let Some(accounts) = self.0.tags.get(&tag) {
            if accounts.is_empty() {
                return Ok(None);
            }
            let address = accounts.random_entry(u)?;
            let account = self
                .0
                .accounts
                .get(address)
                .expect("Account from secondary index must exist");

            Ok(Some((address.clone(), account.into())))
        } else {
            Ok(None)
        }
    }

    fn has_tag(&self, addr: &<S as Spec>::Address, tag: Self::Tag) -> bool {
        self.0
            .tags
            .get(&tag)
            .map(|tag_holders| tag_holders.contains(addr))
            .unwrap_or(false)
    }

    fn get_token(&self, id: &TokenId) -> Option<TokenInfo> {
        self.0.tokens.get(id).cloned()
    }

    fn update_token(&mut self, id: TokenId, info: TokenInfo) {
        self.0.tokens.insert(id, info);
    }

    fn get_random_token(
        &self,
        u: &mut arbitrary::Unstructured<'_>,
    ) -> arbitrary::Result<(TokenId, TokenInfo)> {
        self.0.tokens.random_entry(u).map(|(k, v)| (*k, v.clone()))
    }

    fn update_account(&mut self, address: &S::Address, mut view: Self::AccountView) {
        for action in view.take_tags().into_iter() {
            match action {
                TagAction::Add(tag) => {
                    self.0
                        .tags
                        .entry(tag.into())
                        .or_default()
                        .insert(address.clone());
                }
                TagAction::Remove(tag) => {
                    self.0
                        .tags
                        .get_mut(&tag.into())
                        .map(|set| set.swap_remove(address));
                }
            }
        }
        assert!(
            self.0
                .accounts
                .get_mut(address)
                .map(|account| view.apply_to(account))
                .is_some(),
            "Tried to update account that doesn't exist"
        );
    }

    fn generate_account(
        &mut self,
        u: &mut arbitrary::Unstructured<'_>,
    ) -> arbitrary::Result<(S::Address, Self::AccountView)> {
        let private_key: <<S as Spec>::CryptoSpec as CryptoSpec>::PrivateKey =
            Arbitrary::arbitrary(u)?;
        let address: S::Address = private_key.pub_key().credential_id().into();

        // If an account already exist with that address panic because that means our source of randomness is flawed!
        // There is only a vanishingly small change that two randomly generated addressed are the same
        if self.0.accounts.get(&address).is_some() {
            panic!("The address {address:?} has already been generated. The source of randomness is broken, this is a bug.");
        }

        let account = AccountState::<S, T>::with_private_key(private_key);

        self.0.accounts.insert(address.clone(), account.clone());
        Ok((address, (&account).into()))
    }
}

/// The action to take on a particular tag
#[derive(Debug, Clone)]
pub enum TagAction<T> {
    /// Add the tag to an account.
    Add(T),
    /// Remove the tag from an account, if present.
    Remove(T),
}

impl<T> TagAction<T> {
    /// Apply some function to the tag contained in some [`TagAction`]
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> TagAction<U> {
        match self {
            TagAction::Add(item) => TagAction::Add(f(item)),
            TagAction::Remove(item) => TagAction::Remove(f(item)),
        }
    }
}

/// Try to take an action repeatedly, breaking when the `until` condition is true and executing
/// the `on_failure` expression if unsuccessful after too many attempts.
#[macro_export]
macro_rules! repeatedly {
    (let $assignment:tt = $expression:expr; until: $test:expr, on_failure: $err:expr) => {
        let $assignment = 'repeated: {
            for _ in 0..1_000 {
                let $assignment = $expression;
                if $test {
                    break 'repeated $assignment;
                }
            }
            $err
        };
    };
}

#[cfg(test)]
mod test {
    use arbitrary::Unstructured;
    use rng_utils::get_random_bytes;

    use super::*;

    #[test]
    fn test_uniform_distribution() {
        let distribution = Distribution::equiprobable(10);
        distribution
            .weights_and_values
            .iter()
            .for_each(|(weight, _)| {
                assert_eq!(weight, &Percent(10));
            });
    }

    #[test]
    fn test_distribution_with_values() {
        let distribution = Distribution::with_values(vec![(1, "a"), (0, "b")]);

        let weights_and_values = distribution.weights_and_values.clone();

        assert_eq!(
            weights_and_values.first().unwrap().0,
            Percent::one_hundred()
        );
        assert_eq!(weights_and_values.last().unwrap().0, Percent::zero());

        let data = get_random_bytes(10000, 1);
        let mut u = Unstructured::new(&data);

        for _ in 0..1000 {
            let value = distribution
                .select_value(&mut u)
                .expect("Out of randomness");
            assert_eq!(value, &"a", "The value must be equal to a");
        }
    }
}
