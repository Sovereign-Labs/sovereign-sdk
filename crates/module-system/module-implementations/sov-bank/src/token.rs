use core::str::FromStr;
use std::collections::HashSet;
use std::fmt::{Display, Formatter};
#[cfg(feature = "native")]
use std::num::ParseIntError;

use anyhow::bail;
use borsh::{BorshDeserialize, BorshSerialize};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
#[cfg(feature = "arbitrary")]
use sov_modules_api::prelude::arbitrary;
use sov_modules_api::prelude::*;
use sov_modules_api::transaction::PriorityFeeBips;
use sov_modules_api::{impl_hash32_type, Spec};
use sov_state::{BorshCodec, EncodeLike, StateItemEncoder};
use thiserror::Error;

use crate::utils::{Payable, TokenHolder, TokenHolderRef};
use crate::Amount;

#[derive(Debug, Clone, PartialEq, Eq, BorshDeserialize, BorshSerialize, Serialize, Deserialize)]
/// A token burn rate. We need to burn some of it to avoid the system participants to
/// be incentivized to prove and submit empty blocks.
pub struct BurnRate(u8);

#[derive(Debug, Error)]
#[error("Burn rate must be less than or equal to 100")]
pub struct BurnRateParsingError;

impl BurnRate {
    /// Creates a new burn rate. Panics if the burn rate is greater than 100.
    pub const fn new_unchecked(burn_rate: u8) -> Self {
        // We can panic here since the burn rate is a constant defined at genesis
        if burn_rate > 100 {
            panic!("Burn rate must be less than or equal to 100");
        }

        Self(burn_rate)
    }

    /// Creates a new burn rate from a u8 value.
    /// Since we need a constant function we cannot implement the `TryFrom` trait.
    pub const fn try_from_u8(value: u8) -> Result<Self, BurnRateParsingError> {
        if value > 100 {
            Err(BurnRateParsingError)
        } else {
            Ok(Self(value))
        }
    }

    /// Applies the burn rate to the given amount.
    pub fn apply(&self, amount: Amount) -> Amount {
        let self_as_bips = PriorityFeeBips::from_percentage(100 - self.0 as u64);
        self_as_bips.apply(amount).expect(
            "The final calculation cannot overflow since the burn rate is never greater than 100%",
        )
    }
}

impl_hash32_type!(TokenId, TokenIdBech32, "token_");

/// The key to a `balances` entry, consisting of an Address and TokenID
#[derive(
    Debug,
    Clone,
    PartialEq,
    borsh::BorshDeserialize,
    borsh::BorshSerialize,
    serde::Serialize,
    serde::Deserialize,
    derive_more::Display,
)]
#[display(r#"{}/{}"#, self.0, self.1)]
pub struct BalanceKey<Addr: Display>(pub Addr, pub TokenId);

impl<Addr: Display + BorshSerialize, AddrLike> EncodeLike<(AddrLike, &TokenId), BalanceKey<Addr>>
    for BorshCodec
where
    BorshCodec: EncodeLike<AddrLike, Addr>,
    BorshCodec: StateItemEncoder<TokenId>,
{
    fn encode_like(&self, borrowed: &(AddrLike, &TokenId)) -> Vec<u8> {
        let mut out = self.encode_like(&borrowed.0);
        out.extend_from_slice(&self.encode(borrowed.1));
        out
    }
}

impl<Addr> FromStr for BalanceKey<Addr>
where
    Addr: FromStr<Err: Into<Box<dyn std::error::Error + Send + Sync + 'static>>> + Display,
{
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // The address serialization is unknown to us, so it might contain `/` - but we know that TokenID is
        // bech32 which disallows `/`
        let Some(pos) = s.rfind('/') else {
            bail!("Invalid balance prefix. String does not contain '/'");
        };
        if (pos + 1) == s.len() {
            bail!("Invalid balance prefix. String does not contain token ID");
        }
        let addr = &s[..pos];
        let token = &s[(pos + 1)..];

        Ok(BalanceKey(
            Addr::from_str(addr).map_err(|e| anyhow::Error::from_boxed(e.into()))?,
            TokenId::from_str(token)?,
        ))
    }
}

/// Structure that stores information specifying
/// a given `amount` (type [`Amount`]) of coins stored at a `token_id`
/// (type [`crate::TokenId`]).
#[cfg_attr(feature = "native", derive(clap::Parser))]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
#[derive(
    borsh::BorshDeserialize,
    borsh::BorshSerialize,
    Debug,
    Clone,
    Serialize,
    Deserialize,
    PartialEq,
    Eq,
    JsonSchema,
    UniversalWallet,
)]
#[sov_wallet(show_as = "{} coins of token ID {}")]
pub struct Coins {
    /// The number of tokens
    #[sov_wallet(template("transfer" = input("amount")))]
    #[sov_wallet(fixed_point(from_field(1, offset = 31)))]
    pub amount: Amount,
    /// The ID of the token
    #[sov_wallet(template("transfer" = input("token_id")))]
    pub token_id: TokenId,
}

/// The errors that might arise when parsing a `Coins` struct from a string.
#[cfg(feature = "native")]
#[derive(Debug, Error)]
pub enum CoinsFromStrError {
    /// The amount could not be parsed as an u64.
    #[error("Could not parse {input} as a valid amount: {err}")]
    InvalidAmount { input: String, err: ParseIntError },
    /// The input string was malformed, so the `amount` substring could not be extracted.
    #[error("No amount was provided. Make sure that your input is in the format: amount,token_id. Example: 100,sov15vspj48hpttzyvxu8kzq5klhvaczcpyxn6z6k0hwpwtzs4a6wkvqmlyjd6")]
    NoAmountProvided,
    /// The token ID could not be parsed as a valid address.
    #[error("Could not parse {input} as a valid address: {err}")]
    InvalidTokenAddress { input: String, err: anyhow::Error },
    /// The input string was malformed, so the `token_id` substring could not be extracted.
    #[error("No token ID was provided. Make sure that your input is in the format: amount,token_id. Example: 100,sov15vspj48hpttzyvxu8kzq5klhvaczcpyxn6z6k0hwpwtzs4a6wkvqmlyjd6")]
    NoTokenAddressProvided,
}

#[cfg(feature = "native")]
impl FromStr for Coins {
    type Err = CoinsFromStrError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = s.splitn(2, ',');

        let amount_str = parts.next().ok_or(CoinsFromStrError::NoAmountProvided)?;
        let token_id_str = parts
            .next()
            .ok_or(CoinsFromStrError::NoTokenAddressProvided)?;

        let amount =
            amount_str
                .parse::<Amount>()
                .map_err(|err| CoinsFromStrError::InvalidAmount {
                    input: amount_str.into(),
                    err,
                })?;
        let token_id = TokenId::from_str(token_id_str).map_err(|err| {
            CoinsFromStrError::InvalidTokenAddress {
                input: token_id_str.into(),
                err,
            }
        })?;

        Ok(Self { amount, token_id })
    }
}
impl std::fmt::Display for Coins {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        // implement Display for Coins
        write!(f, "token_id={} amount={}", self.token_id, self.amount)
    }
}

/// This struct represents a token in the sov-bank module.
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq, Clone)]
pub struct Token<S: Spec> {
    /// Name of the token.
    pub(crate) name: String,
    /// Total supply of the coins.
    pub(crate) total_supply: Amount,
    /// The supply cap of the token, if any.
    pub(crate) supply_cap: Amount,

    /// Vector containing the admins
    /// Empty vector indicates that the token supply is frozen.
    /// Non-empty vector indicates members of the vector can mint.
    /// Freezing a token requires emptying the vector
    /// NOTE: This is explicit, so if a creator doesn't add themselves, then they can't mint.
    pub(crate) admins: Vec<TokenHolder<S>>,
}

impl<S: Spec> Token<S> {
    /// Get the name of the token.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Get the total supply of the token.
    pub fn total_supply(&self) -> Amount {
        self.total_supply
    }

    /// Get the admins of the token.
    pub fn admins(&self) -> &[TokenHolder<S>] {
        &self.admins
    }

    /// admins: Vec<Address> is used to determine if the token is frozen or not
    /// If the vector is empty when the function is called, this means the token is already frozen
    pub(crate) fn freeze(&mut self, sender: TokenHolderRef<'_, S>) -> anyhow::Result<()> {
        let sender = sender.as_token_holder();
        if self.admins.is_empty() {
            bail!("Token {} is already frozen", self.name)
        }
        self.assert_is_admin(sender)?;
        self.admins = vec![];
        Ok(())
    }

    pub(crate) fn update_admin(
        &mut self,
        new_admin: Option<S::Address>,
        admin_to_replace: &S::Address,
    ) -> anyhow::Result<()> {
        // Check if the `admin_to_replace` is in the admin list.
        let Some(current_admin_pos) = self.find_admin_index(admin_to_replace) else {
            bail!(
                "Cannot update admin: `{admin_to_replace}` is not in the admin list for the specified token {}",
                self.name
            );
        };

        if let Some(new_admin) = new_admin {
            // The admin list is guaranteed to be unique at genesis.
            // This logic preserves that invariant during updates.

            // 1. If `new_admin` is already in the list, do nothing to avoid duplicates.
            if self.find_admin_index(&new_admin).is_some() {
                bail!("`{new_admin}` is already a member of the admin list.");
            };

            // 2. Replace the `admin_to_replace` with the `new_admin``.
            self.admins[current_admin_pos] = TokenHolder::User(new_admin);
        } else {
            self.admins.remove(current_admin_pos);
        }
        Ok(())
    }

    fn find_admin_index(&self, admin: &S::Address) -> Option<usize> {
        self.admins
            .iter()
            .position(|a| a.as_token_holder() == admin.as_token_holder())
    }

    /// Mints a given `amount` of token sent by `sender` to the specified `mint_to_address`.
    /// Checks that the `admins` set is not empty for the token and that the `sender`
    /// is an `admin`. If so, update the balances of token for the `mint_to_address` by
    /// adding the minted tokens. Updates the `total_supply` of that token.
    pub(crate) fn update_for_mint_if_allowed(
        &mut self,
        authorizer: TokenHolderRef<'_, S>,
        amount: Amount,
    ) -> anyhow::Result<()> {
        if self.admins.is_empty() {
            bail!("Attempt to mint frozen token {}", self.name)
        }

        self.assert_is_admin(authorizer)?;

        let new_supply = self
            .total_supply
            .checked_add(amount)
            .ok_or(anyhow::Error::msg(
                "Total Supply overflow in the mint method of bank module",
            ))?;
        if new_supply > self.supply_cap {
            anyhow::bail!("Attempted to mint more than the supply cap of token. Max supply: {}. Current supply: {}. Minted amount: {}", self.supply_cap, self.total_supply, amount)
        }

        self.total_supply = new_supply;

        Ok(())
    }

    fn assert_is_admin(&self, sender: TokenHolderRef<'_, S>) -> anyhow::Result<()> {
        for minter in self.admins.iter() {
            if sender == minter.as_token_holder() {
                return Ok(());
            }
        }

        bail!("Sender {} is not an admin of token {}", sender, self.name)
    }
}

pub(crate) fn unique_holders<S: Spec>(holders: &[TokenHolderRef<'_, S>]) -> Vec<TokenHolder<S>> {
    // IMPORTANT:
    // We can't just put `admins` into a `HashSet` because the order of the elements in the `HashSet`` is not guaranteed.
    // The algorithm below ensures that the order of the elements in the `auth_minter_list` is deterministic (both in zk and native execution).
    let mut indices = HashSet::new();
    let mut holder_list = Vec::new();

    for item in holders.iter() {
        if indices.insert(item) {
            holder_list.push(item.into());
        }
    }

    holder_list
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use sov_modules_api::{Address, Amount};
    use sov_state::{BorshCodec, EncodeLike, StateItemEncoder};

    use crate::token::Token;
    use crate::{BalanceKey, TokenHolder, TokenId};

    type S = sov_test_utils::TestSpec;

    #[test]
    fn test_balance_key_str_roundtrip() {
        let key: BalanceKey<String> = BalanceKey("Address/".to_string(), TokenId::from([1u8; 32]));
        assert_eq!(
            key.to_string(),
            "Address//token_1qyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqskmlvce"
        );

        assert_eq!(
            BalanceKey::<String>::from_str(&key.to_string()).unwrap(),
            key
        );
    }

    #[test]
    fn test_balance_key_encode_like() {
        let key: BalanceKey<String> = BalanceKey("Address/".to_string(), TokenId::from([1u8; 32]));
        assert_eq!(
            BorshCodec.encode_like(&(key.0.clone(), &key.1)),
            BorshCodec.encode(&key)
        );
    }

    #[test]
    fn test_update_admin() {
        let old_admin = Address::new([22; 28]);
        let new_admin = Address::new([99; 28]);

        let mut token = Token {
            name: "Test".to_string(),
            total_supply: Amount::new(1000),
            supply_cap: Amount::new(10000),
            admins: vec![TokenHolder::<S>::User(old_admin)],
        };

        // Fail becouse new_admin = old_admin
        assert!(token.update_admin(Some(old_admin), &old_admin).is_err());
        // Fail: `new_admin` is not authorized to update the admin list as its not in it.
        assert!(token.update_admin(Some(old_admin), &new_admin).is_err());
        // Success
        assert!(token.update_admin(Some(new_admin), &old_admin).is_ok());
        assert_eq!(token.admins(), &[TokenHolder::<S>::User(new_admin)]);

        assert!(token.update_admin(None, &old_admin).is_err());
        assert!(token.update_admin(None, &new_admin).is_ok());
        assert_eq!(token.admins(), &[]);
    }

    #[test]
    fn test_update_admin_list() {
        let admin_1 = TokenHolder::<S>::User(Address::new([11; 28]));
        let admin_2 = TokenHolder::<S>::User(Address::new([22; 28]));
        let admin_3 = TokenHolder::<S>::User(Address::new([33; 28]));

        let old_admin_addr = Address::new([99; 28]);
        let admins = vec![
            admin_1.clone(),
            TokenHolder::<S>::User(old_admin_addr),
            admin_2.clone(),
            admin_3.clone(),
        ];

        let new_admin_addr = Address::new([77; 28]);

        let mut token = Token {
            name: "Test".to_string(),
            total_supply: Amount::new(1000),
            supply_cap: Amount::new(10000),
            admins,
        };

        // Fail: `new_admin` is the same as `old_admin_addr`, and duplicates are not allowed.
        assert!(token
            .update_admin(Some(old_admin_addr), &old_admin_addr)
            .is_err());
        // Fail: `new_admin` is not authorized to update the admin list.
        assert!(token
            .update_admin(Some(old_admin_addr), &new_admin_addr)
            .is_err());

        // Success: `old_admin_addr` successfully updates the admin list with `new_admin_addr`.
        assert!(token
            .update_admin(Some(new_admin_addr), &old_admin_addr)
            .is_ok());

        assert_eq!(
            token.admins(),
            &vec![
                admin_1.clone(),
                TokenHolder::<S>::User(new_admin_addr),
                admin_2.clone(),
                admin_3.clone()
            ]
        );

        // Fail: `old_admin` has already been replaced by `new_admin`.
        assert!(token.update_admin(None, &old_admin_addr).is_err());
        // Success: `new_admin_addr` successfully removes itself from the admin list.
        assert!(token.update_admin(None, &new_admin_addr).is_ok());
        assert_eq!(token.admins(), &vec![admin_1, admin_2, admin_3]);
    }
}
