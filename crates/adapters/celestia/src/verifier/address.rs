use std::str::FromStr;

use anyhow::Context;
use borsh::{BorshDeserialize, BorshSerialize};
use celestia_types::state::{AccAddress, AddressKind, AddressTrait};
// use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use sov_rollup_interface::reexports::schemars::{self};
use sov_rollup_interface::sov_universal_wallet::UniversalWallet;

#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    PartialOrd,
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
    pub(crate) AccAddress,
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
    fn schema_name() -> String {
        "CelestiaAddress".to_string()
    }

    fn json_schema(_gen: &mut schemars::gen::SchemaGenerator) -> schemars::schema::Schema {
        serde_json::from_value(serde_json::json!({
            "type": "string",
            "pattern": "^celestia[a-z0-9]+$",
            "description": "A Celestia address",
        }))
        .expect("Invalid schema; this is a bug, please report it")
    }
}

impl AsRef<[u8]> for CelestiaAddress {
    fn as_ref(&self) -> &[u8] {
        self.0.id_ref().as_ref()
    }
}

/// Decodes slice of bytes into CelestiaAddress
/// Treats it as string if it starts with HRP and the rest is valid ASCII
/// Otherwise just decodes the tendermint Id and creates address from that.
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

#[cfg(test)]
mod tests {
    use bech32::{Bech32, Hrp};

    const CELESTIA_HRP: Hrp = Hrp::parse_unchecked("celestia");

    use sov_rollup_interface::sov_universal_wallet::schema::Schema;
    use sov_test_utils::validate_schema;

    use super::*;

    #[test]
    fn test_celestia_address_schema() {
        let raw_address_str = ADDR_2;
        let address = CelestiaAddress::from_str(raw_address_str).unwrap();

        let schema = Schema::of_single_type::<CelestiaAddress>().unwrap();

        let borsh_bytes = borsh::to_vec(&address).unwrap();
        let deserialized: CelestiaAddress = borsh::from_slice(&borsh_bytes).unwrap();
        assert_eq!(deserialized, address);

        let displayed_from_schema = schema.display(0, &borsh_bytes).unwrap();
        assert_eq!(&displayed_from_schema, raw_address_str);
    }

    #[test]
    fn test_address_display_from_string() {
        let raw_address_str = ADDR_2;
        let address = CelestiaAddress::from_str(raw_address_str).unwrap();
        let output = format!("{address}");
        assert_eq!(raw_address_str, output);
    }

    #[test]
    fn test_from_string_for_registering() {
        let raw_address_str = "celestia1qursy837n4a97d6q9camret9jtdjff7qtf0yjh";
        let address = CelestiaAddress::from_str(raw_address_str).unwrap();
        let raw_bytes = address.as_ref().to_vec();
        let expected_bytes = vec![
            7, 7, 2, 30, 62, 157, 122, 95, 55, 64, 46, 59, 177, 229, 101, 146, 219, 36, 167, 192,
        ];

        assert_eq!(expected_bytes, raw_bytes);
    }

    #[test]
    fn test_address_display_try_vec() {
        let raw_address_str = ADDR_3;
        let raw_address: Vec<u8> = raw_address_str.bytes().collect();
        let address = CelestiaAddress::try_from(&raw_address[..]).unwrap();
        let output = format!("{address}");
        assert_eq!(raw_address_str, output);
    }

    #[test]
    fn test_from_str_and_from_slice_same() {
        let raw_address_str = ADDR_3;
        let raw_address_array = ADDR_3.as_bytes();

        let address_from_str = CelestiaAddress::from_str(raw_address_str).unwrap();
        let address_from_slice = CelestiaAddress::try_from(raw_address_array).unwrap();

        assert_eq!(address_from_str, address_from_slice);
    }

    // 20 u8 -> 32 u5
    fn check_from_bytes_as_ascii(input: [u8; 20]) {
        let encoded = bech32::encode::<Bech32>(CELESTIA_HRP, &input).unwrap();
        let bytes = encoded.as_bytes();
        let address = CelestiaAddress::try_from(bytes);
        assert!(address.is_ok());
        let address = address.unwrap();
        let output = format!("{address}");
        assert_eq!(encoded, output);
    }

    // 20 u8 -> 32 u5
    fn check_from_as_ref(input: [u8; 20]) {
        let encoded = bech32::encode::<Bech32>(CELESTIA_HRP, &input).unwrap();
        let address1 = CelestiaAddress::from_str(&encoded).unwrap();
        let bytes = address1.as_ref();
        let address = CelestiaAddress::try_from(bytes);
        assert!(address.is_ok());
        let address = address.unwrap();
        let output = format!("{address}");
        assert_eq!(encoded, output);
    }

    #[test_strategy::proptest]
    fn validate_json_schema(input: CelestiaAddress) {
        validate_schema(&input).unwrap();
    }

    #[test_strategy::proptest]
    fn ord_invariants(values: [CelestiaAddress; 3]) {
        reltester::ord(&values[0], &values[1], &values[2]).unwrap();
    }

    #[test_strategy::proptest]
    fn hash_invariants(values: [CelestiaAddress; 2]) {
        reltester::hash(&values[0], &values[1]).unwrap();
    }

    use proptest::sample::size_range;

    use crate::test_helper::{ADDR_2, ADDR_3};

    #[test_strategy::proptest]
    fn test_try_from_any_slice(#[any(size_range(0..100).lift())] input: Vec<u8>) {
        let _ = CelestiaAddress::try_from(&input[..]);
    }

    #[test_strategy::proptest]
    fn test_from_str_anything(#[strategy("\\PC*")] input: String) {
        let _ = CelestiaAddress::from_str(&input);
    }

    #[test_strategy::proptest]
    fn test_from_str_lowercase_ascii(
        // According to spec, alphanumeric characters excluding "1" "b" "i" and "o"
        #[strategy("celestia1[023456789ac-hj-np-z]{38}")] input: String,
    ) {
        let result = CelestiaAddress::from_str(&input);
        if let Ok(address) = result {
            let output = format!("{address}");
            assert_eq!(input, output);
        }
    }

    #[test_strategy::proptest]
    fn test_try_from_ascii_slice(input: [u8; 20]) {
        check_from_bytes_as_ascii(input);
    }

    #[test_strategy::proptest]
    fn test_try_as_ref_from(input: [u8; 20]) {
        check_from_as_ref(input);
    }
}
