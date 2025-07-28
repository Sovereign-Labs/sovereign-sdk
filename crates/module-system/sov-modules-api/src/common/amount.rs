use borsh::{BorshDeserialize, BorshSerialize};
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sov_rollup_interface::sov_universal_wallet::UniversalWallet;
use sov_universal_wallet::ty::IntegerDisplayable;

/// Maximum number of decimal places for a fixed-point number stored as a u128 integer
/// In other words, `log_10(u128::MAX)`
pub const MAX_U128_DECIMAL_PLACES: u8 = 39;

#[derive(
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    BorshDeserialize,
    BorshSerialize,
    JsonSchema,
    UniversalWallet,
    Default,
    derive_more::Debug,
    derive_more::Display,
    Hash,
)]
#[cfg_attr(
    feature = "arbitrary",
    derive(arbitrary::Arbitrary, proptest_derive::Arbitrary)
)]
#[schemars(with = "AmountString")]
#[debug("{}", self.0)]
#[display("{}", self.0)]
/// A token amount.
pub struct Amount(pub u128);

impl Amount {
    #[allow(missing_docs)]
    pub const MAX: Amount = Amount(u128::MAX);
    #[allow(missing_docs)]
    pub const MIN: Amount = Amount(u128::MIN);
    #[allow(missing_docs)]
    pub const ZERO: Amount = Amount(0);
    /// Maximum number of decimals allowed for fixed-point representation of `Amount`s.
    pub const MAX_DECIMALS: u8 = MAX_U128_DECIMAL_PLACES;
    #[allow(missing_docs)]
    pub const fn new(amount: u128) -> Self {
        Self(amount)
    }
}

// This helper struct isn't actually dead. It's used by Schemars, which rustc doesn't detect.
#[allow(dead_code)]
#[derive(JsonSchema)]
struct AmountString(#[schemars(regex(pattern = "^[0-9]+$"))] String);

impl IntegerDisplayable for Amount {
    fn integer_type() -> sov_universal_wallet::ty::IntegerType {
        sov_universal_wallet::ty::IntegerType::u128
    }
}

impl Amount {
    /// Checked addition.
    pub fn checked_add(&self, other: Amount) -> Option<Amount> {
        self.0.checked_add(other.0).map(Amount)
    }

    /// Checked subtraction.
    pub fn checked_sub(&self, other: Amount) -> Option<Amount> {
        self.0.checked_sub(other.0).map(Amount)
    }

    /// Checked multiplication.
    pub fn checked_mul(&self, other: Amount) -> Option<Amount> {
        self.0.checked_mul(other.0).map(Amount)
    }
    /// Checked division
    pub fn checked_div(&self, other: Amount) -> Option<Amount> {
        self.0.checked_div(other.0).map(Amount)
    }
    /// Saturating addition
    pub fn saturating_add(&self, other: Amount) -> Amount {
        Amount(self.0.saturating_add(other.0))
    }
    /// Saturating subtraction
    pub fn saturating_sub(&self, other: Amount) -> Amount {
        Amount(self.0.saturating_sub(other.0))
    }
    /// Saturating multiplication
    pub fn saturating_mul(&self, other: Amount) -> Amount {
        Amount(self.0.saturating_mul(other.0))
    }
    /// Saturating division
    pub fn saturating_div(&self, other: Amount) -> Amount {
        Amount(self.0.saturating_div(other.0))
    }
}

// Implement the parsing traits needed by clap
impl std::str::FromStr for Amount {
    type Err = <u128 as std::str::FromStr>::Err;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse::<u128>().map(Self)
    }
}

impl PartialEq<u128> for Amount {
    fn eq(&self, other: &u128) -> bool {
        self.0 == *other
    }
}

impl PartialOrd<u128> for Amount {
    fn partial_cmp(&self, other: &u128) -> Option<std::cmp::Ordering> {
        self.0.partial_cmp(other)
    }
}

impl From<u128> for Amount {
    fn from(amount: u128) -> Self {
        Self(amount)
    }
}

impl From<u64> for Amount {
    fn from(amount: u64) -> Self {
        Self(u128::from(amount))
    }
}

impl Serialize for Amount {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if serializer.is_human_readable() {
            // For human-readable formats (e.g. JSON, YAML):
            // Serialize as a string.
            serializer.serialize_str(&self.0.to_string())
        } else {
            // For binary formats (e.g. Bincode):
            // Serialize as a u128.
            serializer.serialize_u128(self.0)
        }
    }
}

impl<'de> Deserialize<'de> for Amount {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        if deserializer.is_human_readable() {
            let s = <String as serde::Deserialize>::deserialize(deserializer)?;
            let val = s.parse::<u128>().map_err(serde::de::Error::custom)?;
            Ok(Amount(val))
        } else {
            let val = <u128 as serde::Deserialize>::deserialize(deserializer)?;
            Ok(Amount(val))
        }
    }
}
