use std::str::FromStr;

use anyhow::Context;
use borsh::{BorshDeserialize, BorshSerialize};
use celestia_types::state::{AccAddress, AddressKind, AddressTrait};
// use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use sov_rollup_interface::reexports::schemars::{self};
use sov_universal_wallet::UniversalWallet;

#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    PartialOrd,
    Copy,
    Ord,
    Serialize,
    Deserialize,
    Hash,
    derive_more::Display,
    BorshSerialize,
    BorshDeserialize,
    UniversalWallet,
)]
pub struct CelestiaAddress(
    #[borsh(
        serialize_with = "serialize_celestia_address",
        deserialize_with = "deserialize_celestia_address"
    )]
    #[sov_wallet(as_ty = "CelestiaAddressSchema")]
    pub AccAddress,
);

#[cfg(feature = "arbitrary")]
mod arbitrary_impls {
    use prop::arbitrary::any;
    use prop::strategy::Strategy;
    use proptest::prelude::prop;
    use proptest::strategy::BoxedStrategy;

    use super::*;

    fn new(bytes: [u8; 20]) -> CelestiaAddress {
        CelestiaAddress(AccAddress::new(tendermint::account::Id::new(bytes)))
    }

    impl<'a> ::arbitrary::Arbitrary<'a> for CelestiaAddress {
        fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
            let bytes = <[u8; 20]>::arbitrary(u)?;
            Ok(new(bytes))
        }
    }

    #[cfg(feature = "arbitrary")]
    impl proptest::arbitrary::Arbitrary for CelestiaAddress {
        type Parameters = ();
        fn arbitrary_with((): Self::Parameters) -> Self::Strategy {
            any::<[u8; 20]>().prop_map(new).boxed()
        }

        type Strategy = BoxedStrategy<Self>;
    }
}

const CELESTIA: &str = "celestia";
#[derive(UniversalWallet)]
#[allow(dead_code)]
#[doc(hidden)]
struct CelestiaAddressSchema(#[sov_wallet(display(bech32(prefix = "CELESTIA")))] Vec<u8>);

fn serialize_celestia_address(
    address: &AccAddress,
    writer: &mut impl borsh::io::Write,
) -> Result<(), borsh::io::Error> {
    let id = address.id_ref();
    BorshSerialize::serialize(id.as_bytes(), writer)
}

fn deserialize_celestia_address(
    reader: &mut impl borsh::io::Read,
) -> Result<AccAddress, borsh::io::Error> {
    let bytes: Vec<u8> = BorshDeserialize::deserialize_reader(reader)?;
    let id =
        bytes
            .try_into()
            .map_err(|e: <Vec<u8> as TryInto<tendermint::account::Id>>::Error| {
                std::io::Error::other(e.to_string())
            })?;
    Ok(AccAddress::new(id))
}

impl schemars::JsonSchema for CelestiaAddress {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "CelestiaAddress".into()
    }

    fn json_schema(_gen: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "pattern": "^celestia[a-z0-9]+$",
            "description": "A Celestia address",
        })
    }
}

impl AsRef<[u8]> for CelestiaAddress {
    fn as_ref(&self) -> &[u8] {
        self.0.id_ref().as_ref()
    }
}

/// Decodes bytes into `CelestiaAddress`.
/// If input is ASCII and starts with the account HRP, parses it as bech32 text.
/// Otherwise expects a raw 20-byte Tendermint account ID.
/// Note: HRP-prefixed ASCII that fails bech32 parsing returns an error (no raw-ID fallback).
impl<'a> TryFrom<&'a [u8]> for CelestiaAddress {
    type Error = anyhow::Error;

    fn try_from(value: &'a [u8]) -> Result<Self, Self::Error> {
        let hrp = AddressKind::Account.prefix();

        if value.starts_with(hrp.as_bytes()) && value.is_ascii() {
            // safety, because we checked that it is ASCII
            let s = unsafe { std::str::from_utf8_unchecked(value) };
            s.parse().context("failed parsing celestia address")
        } else {
            let array = value.try_into().context("invalid slice length")?;
            let id = tendermint::account::Id::new(array);
            Ok(Self(AccAddress::new(id)))
        }
    }
}

impl FromStr for CelestiaAddress {
    type Err = <AccAddress as FromStr>::Err;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(s.parse()?))
    }
}

impl sov_rollup_interface::BasicAddress for CelestiaAddress {}
