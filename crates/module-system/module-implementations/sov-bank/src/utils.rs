use std::hash::Hash;
use std::str::FromStr;

use borsh::BorshDeserialize;
use sov_modules_api::digest::Digest;
use sov_modules_api::{CryptoSpec, MeteredHasher, ModuleId, Spec, TxState};
use sov_state::codec::{BcsCodec, BorshCodec, EncodeLike};

use crate::derived_holder::DerivedHolder;
use crate::{TokenId, DEFAULT_TOKEN_DECIMALS};

/// Derives token ID from `token_name` and `originator`
pub fn get_token_id<S: Spec>(
    token_name: &str,
    token_decimals: Option<u8>,
    originator: impl Payable<S>,
) -> TokenId {
    let token_decimals = token_decimals.unwrap_or(DEFAULT_TOKEN_DECIMALS);
    let mut hasher = <S::CryptoSpec as CryptoSpec>::Hasher::new();
    hasher.update(originator.as_token_holder().as_bytes());
    hasher.update(token_name.as_bytes());
    hasher.update(token_decimals.to_le_bytes());

    let mut hash: [u8; 32] = hasher.finalize().into();
    hash[31] = token_decimals;
    TokenId::from(hash)
}

/// Derives token ID from `token_name`, `token_decimals` and `originator` while tracking gas usage
/// associated with hashing operations.
/// The decimals are also encoded in the last byte of the hash.
pub fn get_token_id_metered<S: Spec>(
    token_name: &str,
    token_decimals: Option<u8>,
    originator: &impl Payable<S>,
    state: &mut impl TxState<S>,
) -> anyhow::Result<TokenId> {
    let token_decimals = token_decimals.unwrap_or(DEFAULT_TOKEN_DECIMALS);
    let mut hasher = MeteredHasher::<_, <S::CryptoSpec as CryptoSpec>::Hasher>::new(state);
    hasher.update(originator.as_token_holder().as_bytes())?;
    hasher.update(token_name.as_bytes())?;
    hasher.update(&token_decimals.to_le_bytes())?;

    let mut hash: [u8; 32] = hasher.finalize().map_err(|(_, e)| e)?;
    hash[31] = token_decimals;
    Ok(TokenId::from(hash))
}

/// An identifier which can hold tokens on the rollup. This is implemented by `&S::Address`. To pay a module,
/// make sure the `AsPayable` trait is in scope, and call `module_id.to_payable()`.
///
/// When a function accepts `impl Payable<S>` as an argument, you can pass `S::Address` or `ModuleId`, or (to avoid copying)
/// `module_id.as_token_holder()`
pub trait Payable<S: Spec>: std::fmt::Display {
    /// Converts the identifier into a standard format understood by the bank module.
    fn as_token_holder(&self) -> TokenHolderRef<'_, S>;
}

/// A type which can be converted to a type that implements `Payable<S>`. Usually a `ModuleId`.
pub trait IntoPayable<S: Spec> {
    /// A type which implements `Payable<S>` that can be constructed from the current type.
    type Output<'a>: Payable<S>
    where
        Self: 'a;
    /// Converts the
    fn to_payable(&self) -> Self::Output<'_>;
}

impl<S: Spec> Payable<S> for &S::Address {
    fn as_token_holder(&self) -> TokenHolderRef<'_, S> {
        TokenHolderRef::User(*self)
    }
}

impl<S: Spec> IntoPayable<S> for ModuleId {
    type Output<'a> = TokenHolderRef<'a, S>;
    fn to_payable(&self) -> TokenHolderRef<'_, S> {
        TokenHolderRef::Module(self)
    }
}

impl<S: Spec> IntoPayable<S> for DerivedHolder {
    type Output<'a> = TokenHolderRef<'a, S>;
    fn to_payable(&self) -> TokenHolderRef<'_, S> {
        TokenHolderRef::Derived(self)
    }
}

impl<S: Spec> Payable<S> for TokenHolder<S> {
    fn as_token_holder(&self) -> TokenHolderRef<'_, S> {
        match self {
            Self::User(addr) => TokenHolderRef::User(addr),
            Self::Module(id) => TokenHolderRef::Module(id),
            Self::Derived(dh) => TokenHolderRef::Derived(dh),
        }
    }
}

impl<'a, S: Spec> Payable<S> for TokenHolderRef<'a, S> {
    fn as_token_holder(&self) -> TokenHolderRef<'a, S> {
        *self
    }
}

#[derive(
    Debug,
    Clone,
    Eq,
    PartialEq,
    serde::Deserialize,
    BorshDeserialize,
    derive_more::Display,
    schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
#[schemars(bound = "S: Spec", rename = "TokenHolder")]
#[schemars(untagged)]
/// The identifier of a a payable entity on the rollup. This can be either a user or a module.
pub enum TokenHolder<S: Spec> {
    /// A external address the rollup.
    User(S::Address),
    /// A builtin module.
    Module(ModuleId),
    /// A programmatically generated holder.
    Derived(DerivedHolder),
}

impl<S: Spec> FromStr for TokenHolder<S> {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.starts_with(ModuleId::bech32_prefix()) {
            return Ok(Self::Module(ModuleId::from_str(s)?));
        } else if s.starts_with(DerivedHolder::bech32_prefix()) {
            return Ok(Self::Derived(DerivedHolder::from_str(s)?));
        }
        Ok(Self::User(
            S::Address::from_str(s).map_err(|e| anyhow::Error::from_boxed(e.into()))?,
        ))
    }
}

impl<Sp: Spec> serde::Serialize for TokenHolder<Sp> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let id_ref: TokenHolderRef<'_, Sp> = self.into();
        id_ref.serialize(serializer)
    }
}

impl<S: Spec> borsh::BorshSerialize for TokenHolder<S> {
    fn serialize<W: std::io::prelude::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        let id_ref: TokenHolderRef<'_, S> = self.into();
        borsh::BorshSerialize::serialize(&id_ref, writer)
    }
}

#[derive(Debug, serde::Serialize, borsh::BorshSerialize, derive_more::Display)]
#[serde(rename_all = "snake_case")]
/// A reference to a payable entity on the rollup. This can be either a user or a module.
pub enum TokenHolderRef<'a, S: Spec> {
    /// A reference to a user's address
    User(&'a S::Address),
    /// A reference to a module's ID
    Module(&'a ModuleId),
    /// A reference to a programmatically generated holder.
    Derived(&'a DerivedHolder),
}

impl<S: Spec> TokenHolderRef<'_, S> {
    /// Converts `TokenHolderRef` to byte slice.
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            TokenHolderRef::User(addr) => addr.as_ref(),
            TokenHolderRef::Module(id) => id.as_ref(),
            TokenHolderRef::Derived(dh) => dh.as_ref(),
        }
    }

    /// Converts a [`TokenHolderRef`] to an owned [`TokenHolder`].
    pub fn to_owned(&self) -> TokenHolder<S> {
        match self {
            TokenHolderRef::User(addr) => TokenHolder::User((*addr).clone()),
            TokenHolderRef::Module(id) => TokenHolder::Module(**id),
            TokenHolderRef::Derived(dh) => TokenHolder::Derived(**dh),
        }
    }
}

impl<'a, S: Spec> From<&TokenHolderRef<'a, S>> for TokenHolder<S> {
    fn from(item: &TokenHolderRef<'a, S>) -> Self {
        match item {
            TokenHolderRef::User(addr) => TokenHolder::User((*addr).clone()),
            TokenHolderRef::Module(id) => TokenHolder::Module(**id),
            TokenHolderRef::Derived(dh) => TokenHolder::Derived(**dh),
        }
    }
}

impl<'a, S: Spec> From<TokenHolderRef<'a, S>> for TokenHolder<S> {
    fn from(item: TokenHolderRef<'a, S>) -> Self {
        Self::from(&item)
    }
}

impl<S: Spec> Hash for TokenHolderRef<'_, S> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match self {
            Self::User(addr) => {
                state.write_u8(0);
                addr.hash(state);
            }
            Self::Module(id) => {
                state.write_u8(1);
                id.hash(state);
            }
            Self::Derived(dh) => {
                state.write_u8(2);
                dh.hash(state);
            }
        }
    }
}

impl<S: Spec> PartialEq for TokenHolderRef<'_, S> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::User(a), Self::User(b)) => a == b,
            (Self::Module(a), Self::Module(b)) => a == b,
            (Self::Derived(a), Self::Derived(b)) => a == b,
            _ => false,
        }
    }
}

impl<S: Spec> Eq for TokenHolderRef<'_, S> {}

// Manually implement Clone because derive infurs a spurious `Spec: Clone` bound
impl<S: Spec> Clone for TokenHolderRef<'_, S> {
    fn clone(&self) -> Self {
        *self
    }
}

// Manually implement Copy because derive infurs a spurious `Spec: Copy` bound
impl<S: Spec> Copy for TokenHolderRef<'_, S> {}

impl<'a, S: Spec> From<&'a TokenHolder<S>> for TokenHolderRef<'a, S> {
    fn from(item: &'a TokenHolder<S>) -> TokenHolderRef<'a, S> {
        match item {
            TokenHolder::User(addr) => TokenHolderRef::User(addr),
            TokenHolder::Module(id) => TokenHolderRef::Module(id),
            TokenHolder::Derived(dh) => TokenHolderRef::Derived(dh),
        }
    }
}

// use the autoref trick to prevent conflicts since rustc doesn't know that S::Address
// cannot be the same type as ModuleId
impl<'a, S: Spec> From<&&'a S::Address> for TokenHolderRef<'a, S> {
    fn from(value: &&'a S::Address) -> Self {
        Self::User(*value)
    }
}

impl<'a, S: Spec> From<&'a ModuleId> for TokenHolderRef<'a, S> {
    fn from(value: &'a ModuleId) -> Self {
        Self::Module(value)
    }
}

impl<'a, S: Spec> From<&'a DerivedHolder> for TokenHolderRef<'a, S> {
    fn from(value: &'a DerivedHolder) -> Self {
        Self::Derived(value)
    }
}

// Implement the [`EncodeLike`] trait, marking for Rustc that [`TokenHolderRef`]
// and [`TokenHolder`] can be serialized identically for all of our supported
// codecs.
mod encode_like {
    use sov_state::StateItemEncoder;

    use super::*;

    impl<S: Spec> EncodeLike<TokenHolderRef<'_, S>, TokenHolder<S>> for BcsCodec {
        fn encode_like(&self, borrowed: &TokenHolderRef<'_, S>) -> Vec<u8> {
            self.encode(borrowed)
        }
    }

    impl<S: Spec> EncodeLike<TokenHolderRef<'_, S>, TokenHolder<S>> for BorshCodec {
        fn encode_like(&self, borrowed: &TokenHolderRef<'_, S>) -> Vec<u8> {
            self.encode(borrowed)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    use derive_more::FromStr;
    use sov_modules_api::default_spec::DefaultSpec;
    use sov_modules_api::execution_mode::Native;
    use sov_test_utils::{MockDaSpec, MockZkvm};

    type S = DefaultSpec<MockDaSpec, MockZkvm, MockZkvm, Native>;
    use super::*;

    fn calculate_hash<T: Hash>(t: &T) -> u64 {
        let mut hasher = DefaultHasher::new();
        t.hash(&mut hasher);
        hasher.finish()
    }

    #[test]
    fn check_hashes_for_token_holders() {
        let module_id_source: [u8; 32] = [0; 32];
        let address_source: [u8; 28] = [0; 28];

        let module_id = ModuleId::from(module_id_source);
        let module_id_ref: TokenHolderRef<'_, S> = TokenHolderRef::from(&module_id);

        let address = &<S as Spec>::Address::from(address_source);
        let address_ref: TokenHolderRef<'_, S> = TokenHolderRef::from(&address);

        let address_hash = calculate_hash(&address_ref);
        let module_id_hash = calculate_hash(&module_id_ref);

        assert_ne!(
            address_hash, module_id_hash,
            "Hashes for module id and address derived from same source should be different"
        );
    }

    #[test]
    fn test_derived_holder() {
        let source: [u8; 32] = [0; 32];

        let original_dh = DerivedHolder::from(source);
        assert_eq!(
            original_dh.to_string(),
            "derived_1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqgfk7l6"
        );

        let dh = DerivedHolder::from_str(&original_dh.to_string()).unwrap();
        assert_eq!(original_dh, dh);

        let token_holder = TokenHolderRef::<S>::from(&original_dh);
        let payable = original_dh.to_payable();

        assert_eq!(token_holder, payable);
    }
}
