use std::ops::{Deref, DerefMut};

use sov_universal_wallet::UniversalWallet;

// Needed for UniversalWallet derive macro because we are inside the
// sov_rollup_interface crate.
use crate as sov_rollup_interface;

/// Uniquely identifies a slot **within the canonical DA fork**.
///
/// Slots across reorgs can have the same [`SlotNumber`].
#[derive(
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    derive_more::FromStr,
    derive_more::Add,
    derive_more::Sub,
    derive_more::Display,
    serde::Serialize,
    serde::Deserialize,
    borsh::BorshDeserialize,
    borsh::BorshSerialize,
    UniversalWallet,
)]
#[cfg_attr(
    feature = "arbitrary",
    derive(arbitrary::Arbitrary, proptest_derive::Arbitrary)
)]
pub struct SlotNumber(u64);

impl std::fmt::Debug for SlotNumber {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.get())
    }
}

impl SlotNumber {
    /// The very first [`SlotNumber`] of the rollup.
    pub const GENESIS: Self = Self(0);

    /// The [`SlotNumber`] after genesis.
    pub const ONE: Self = Self(1);

    /// The largest possible [`SlotNumber`].
    pub const MAX: Self = Self(u64::MAX);

    /// Wraps a [`u64`] into a [`SlotNumber`].
    pub const fn new(height: u64) -> Self {
        Self(height)
    }

    /// Increments by one and also returns the new [`SlotNumber`].
    pub fn incr(&mut self) -> Self {
        *self = self.next();
        *self
    }

    /// Decrements by one and also returns the new [`SlotNumber`].
    pub fn decr(&mut self) -> Self {
        *self = self.prev();
        *self
    }

    /// The current [`SlotNumber`] plus one.
    ///
    /// # Panics
    ///
    /// Panics in case of overflow.
    pub fn next(&self) -> Self {
        Self(self.0.checked_add(1).unwrap())
    }

    /// The current [`SlotNumber`] minus one.
    ///
    /// # Panics
    ///
    /// Panics if the result underflows.
    pub fn prev(&self) -> Self {
        Self(self.0.checked_sub(1).unwrap())
    }

    /// The current [`SlotNumber`] as a [`u64`].
    pub fn get(&self) -> u64 {
        self.0
    }

    /// Constructs a [`SlotNumber`] from a [`u64`]. This method should be used with caution,
    /// since passing a [`SlotNumber`] that is not a valid height can lead to unexpected results.
    pub fn new_dangerous(height: u64) -> Self {
        Self(height)
    }

    /// Calculates the difference between two [`SlotNumber`]s as a [`u64`].
    ///
    /// # Panics
    ///
    /// Panics if the result underflows.
    pub fn delta(&self, rhs: Self) -> u64 {
        self.0.checked_sub(rhs.0).unwrap()
    }

    /// Calculates the difference between two [`SlotNumber`]s as a [`u64`].
    ///
    /// # Return
    ///
    /// Returns `None` if the result would have underflowed.
    pub fn saturating_delta(&self, rhs: Self) -> u64 {
        self.0.saturating_sub(rhs.0)
    }

    /// See [`u64::saturating_add`].
    pub fn saturating_add(&self, rhs: u64) -> Self {
        Self(self.0.saturating_add(rhs))
    }

    /// See [`u64::saturating_sub`].
    pub fn saturating_sub(&self, rhs: u64) -> Self {
        Self(self.0.saturating_sub(rhs))
    }

    /// See [`u64::checked_add`].
    pub fn checked_add(&self, rhs: u64) -> Option<Self> {
        self.0.checked_add(rhs).map(Self)
    }
    /// See [`u64::checked_sub`].
    pub fn checked_sub(&self, rhs: u64) -> Option<Self> {
        self.0.checked_sub(rhs).map(Self)
    }
    /// Casts this value into a [`SlotNumber`].
    ///
    /// <div class="warning">
    /// This type cast is NEVER safe (as far as I can tell; I don't see edge
    /// cases in which we'd truly want to do this). All usages of this method
    /// should be reviewed AND removed.
    /// </div>
    pub fn as_visible(&self) -> VisibleSlotNumber {
        VisibleSlotNumber::new_dangerous(self.get())
    }

    /// Iterates over all [`SlotNumber`]s in the range `[self, end]`.
    pub fn range_inclusive(&self, end: Self) -> impl Iterator<Item = Self> {
        (self.get()..=end.get()).map(Self::new)
    }

    /// Iterates over all [`SlotNumber`]s in the range `[self, end)`.
    pub fn range_exclusive(&self, end: Self) -> impl Iterator<Item = Self> {
        (self.get()..end.get()).map(Self::new)
    }
}

/// Easy initialization of [`SlotNumber`] and [`VisibleSlotNumber`].
pub trait IntoSlotNumber {
    /// Creates a new [`SlotNumber`].
    fn to_slot_number(self) -> SlotNumber;

    /// Creates a new [`VisibleSlotNumber`].
    fn to_visible_slot_number(self) -> VisibleSlotNumber;
}

macro_rules! impl_into_slot_number {
    ($t:ty) => {
        impl IntoSlotNumber for $t {
            fn to_slot_number(self) -> SlotNumber {
                SlotNumber::new(self as _)
            }

            fn to_visible_slot_number(self) -> VisibleSlotNumber {
                VisibleSlotNumber::new_dangerous(self as _)
            }
        }
    };
}

impl_into_slot_number!(u8);
impl_into_slot_number!(i32);
impl_into_slot_number!(u32);
impl_into_slot_number!(u64);
impl_into_slot_number!(usize);

/// A [`SlotNumber`] at which a user-space state transition happened.
///
/// A [`VisibleSlotNumber`] can safely be type cast into a [`SlotNumber`] to
/// obtain a valid [`SlotNumber`], but the reverse is not true.
#[derive(
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    derive_more::FromStr,
    derive_more::Add, // TODO(@nesofu): remove this, as it's not safe.
    derive_more::Display,
    serde::Serialize,
    serde::Deserialize,
    borsh::BorshDeserialize,
    borsh::BorshSerialize,
    UniversalWallet,
)]
#[cfg_attr(
    feature = "arbitrary",
    derive(arbitrary::Arbitrary, proptest_derive::Arbitrary)
)]
pub struct VisibleSlotNumber(SlotNumber);

impl std::fmt::Debug for VisibleSlotNumber {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.get())
    }
}

impl Deref for VisibleSlotNumber {
    type Target = SlotNumber;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for VisibleSlotNumber {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl VisibleSlotNumber {
    /// At genesis, [`VisibleSlotNumber`] is equal to [`SlotNumber`].
    pub const GENESIS: Self = Self(SlotNumber::GENESIS);

    /// The [`VisibleSlotNumber`] after genesis.
    pub const ONE: Self = Self(SlotNumber::ONE);

    /// The largest possible [`VisibleSlotNumber`].
    pub const MAX: Self = Self(SlotNumber::MAX);

    /// Wraps a [`u64`] into a [`VisibleSlotNumber`].
    /// This method should be used with caution,
    /// since passing a [`VisibleSlotNumber`] that is not a valid height can lead to unexpected results.
    pub fn new_dangerous(height: u64) -> Self {
        Self(SlotNumber::new(height))
    }

    /// Increments by a certain `amount` and also returns the new
    /// [`VisibleSlotNumber`].
    ///
    /// # Panics
    ///
    /// Panics in case of overflow.
    pub fn advance(&mut self, amount: u64) -> Self {
        self.0 = self.0.checked_add(amount).unwrap();
        *self
    }

    /// Casts this value into a [`SlotNumber`].
    ///
    /// <div class="warning">
    /// Type casting between [`SlotNumber`] and [`VisibleSlotNumber`] is not
    /// encouraged as it can easily lead to bugs. TODO(@neysofu): carefully
    /// review all type casts and possibly remove this method.
    /// </div>
    pub fn as_true(&self) -> SlotNumber {
        self.0
    }
}

#[test]
fn test_visible_slot_number() {
    let mut number = VisibleSlotNumber::GENESIS;
    let output = number.advance(5);
    assert_eq!(number.get(), 5);
    assert_eq!(output.get(), 5);
}

/// A rollup "block number". Rollup heights increase in order (1, 2, 3, ...),
/// regardless of what happens on the underlying DA layer.
#[derive(
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    derive_more::Display,
    derive_more::FromStr,
    serde::Serialize,
    serde::Deserialize,
    borsh::BorshDeserialize,
    borsh::BorshSerialize,
)]
pub struct RollupHeight(u64);

impl std::fmt::Debug for RollupHeight {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.get())
    }
}

impl RollupHeight {
    /// The genesis rollup height.
    pub const GENESIS: Self = Self(0);

    /// The height of the first rollup block after genesis.
    pub const ONE: Self = Self(1);

    /// Create a new rollup height from a u64.
    pub fn new(height: u64) -> Self {
        Self(height)
    }

    /// Returns the inner value of a rollup height.
    pub fn get(&self) -> u64 {
        self.0
    }

    /// Increment a rollup height by one.
    pub fn incr(&mut self) {
        self.0 += 1;
    }

    /// See [`u64::checked_sub`]
    #[must_use]
    pub fn checked_sub(self, rhs: u64) -> Option<Self> {
        self.0.checked_sub(rhs).map(Self)
    }

    /// See [`u64::checked_add`]
    #[must_use]
    pub fn checked_add(self, rhs: u64) -> Option<Self> {
        self.0.checked_add(rhs).map(Self)
    }

    /// See [`u64::saturating_sub`]
    #[must_use]
    pub fn saturating_sub(self, rhs: u64) -> Self {
        Self(self.0.saturating_sub(rhs))
    }

    /// See [`u64::saturating_add`]
    #[must_use]
    pub fn saturating_add(self, rhs: u64) -> Self {
        Self(self.0.saturating_add(rhs))
    }

    /// Convert a rollup height to a slot number
    #[must_use]
    pub fn to_slot_number(&self) -> SlotNumber {
        SlotNumber::new_dangerous(self.0)
    }
}
