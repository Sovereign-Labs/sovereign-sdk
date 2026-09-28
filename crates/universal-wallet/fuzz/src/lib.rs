use arbitrary::{Arbitrary, Unstructured};
use borsh::{BorshDeserialize, BorshSerialize};
use sov_modules_api::macros::UniversalWallet;
use sov_modules_api::prelude::serde::{Deserialize, Serialize};
use sov_modules_api::SafeString;
use sov_universal_wallet::schema::safe_string::DEFAULT_MAX_STRING_LENGTH;

#[cfg(feature = "js-compat")]
mod js_compat;

#[cfg(feature = "js-compat")]
pub mod types {
    pub type I64 = crate::js_compat::JsI64;
    pub type I128 = crate::js_compat::JsI128;
    pub type U64 = crate::js_compat::JsU64;
    pub type U128 = crate::js_compat::JsU128;
    pub type F32 = crate::js_compat::JsF32;
    pub type F64 = crate::js_compat::JsF64;
}

#[cfg(not(feature = "js-compat"))]
pub mod types {
    pub type I64 = i64;
    pub type I128 = i128;
    pub type U64 = u64;
    pub type U128 = u128;
    pub type F32 = f32;
    pub type F64 = f64;
}

use types::{F32, F64, I128, I64, U128, U64};

// arbitrary isn't implemented for safe string
#[derive(Debug, BorshSerialize, BorshDeserialize, Serialize, Deserialize, UniversalWallet)]
pub struct ArbitrarySafeString(SafeString);

impl Arbitrary<'_> for ArbitrarySafeString {
    fn arbitrary(u: &mut Unstructured<'_>) -> arbitrary::Result<Self> {
        let len = u.int_in_range(0..=DEFAULT_MAX_STRING_LENGTH)?;

        let chars: Result<String, _> = (0..len)
            .map(|_| {
                let c = u.int_in_range(32u8..=126u8)? as char;
                Ok(c)
            })
            .collect();

        let s = chars?;
        Ok(ArbitrarySafeString(s.try_into().unwrap()))
    }
}

fn bech_prefix() -> &'static str {
    "test"
}

#[derive(
    Debug, BorshSerialize, BorshDeserialize, Serialize, Deserialize, UniversalWallet, Arbitrary,
)]
pub enum ByteVecInput {
    Hex(#[sov_wallet(display(hex))] Vec<u8>),
    Base58 {
        #[sov_wallet(display(base58))]
        address: Vec<u8>,
    },
    Decimal(#[sov_wallet(display(decimal))] Vec<u8>),
    Bechm(#[sov_wallet(display(bech32m(prefix = "bech_prefix()")))] Vec<u8>),
}

#[derive(
    Debug, BorshSerialize, BorshDeserialize, Serialize, Deserialize, UniversalWallet, Arbitrary,
)]
pub enum ByteArrayInput {
    Hex(#[sov_wallet(display(hex))] [u8; 32]),
    Base58(#[sov_wallet(display(base58))] [u8; 32]),
    Decimal(#[sov_wallet(display(decimal))] [u8; 32]),
    Bech(#[sov_wallet(display(bech32(prefix = "bech_prefix()")))] [u8; 32]),
}

#[derive(
    Debug, BorshSerialize, BorshDeserialize, Serialize, Deserialize, UniversalWallet, Arbitrary,
)]
pub enum NumberInput {
    U8(u8),
    U16(u16),
    U32(u32),
    U64(U64),
    U128(U128),
    I8(i8),
    I16(i16),
    I32(i32),
    I64(I64),
    I128(I128),
    F32(F32),
    F64(F64),
}

#[derive(
    Debug, BorshSerialize, BorshDeserialize, Serialize, Deserialize, UniversalWallet, Arbitrary,
)]
pub struct ComplexStruct {
    field_a: Vec<(Option<u8>, [i8; 32])>,
    needs_more_vecs: Vec<Vec<Vec<Vec<U128>>>>,
    never: (),
    bulk_tuple: (i32, i32, U64, I128, ArbitrarySafeString, Option<bool>),
}

#[derive(
    Debug, BorshSerialize, BorshDeserialize, Serialize, Deserialize, UniversalWallet, Arbitrary,
)]
pub struct SkippedField {
    #[allow(dead_code)]
    #[borsh(skip)]
    #[serde(skip)]
    #[sov_wallet(skip)]
    skipper: u8,
    not_skipped: u8,
}

/// A multi-field tuple struct: its schema records `type_name: Some("NamedTupleStruct")`.
#[derive(
    Debug, BorshSerialize, BorshDeserialize, Serialize, Deserialize, UniversalWallet, Arbitrary,
)]
pub struct NamedTupleStruct(i8, U64, ArbitrarySafeString);

/// A newtype: its schema records `type_name: Some("NewtypeStruct")`, but it stays transparent
/// for display and JSON exactly like an anonymous single-field tuple.
#[derive(
    Debug, BorshSerialize, BorshDeserialize, Serialize, Deserialize, UniversalWallet, Arbitrary,
)]
pub struct NewtypeStruct(u32);

/// A tuple struct that opts out of name recording: its schema has `type_name: None`, identical
/// to that of an anonymous `(i16, Option<u8>)`.
#[derive(
    Debug, BorshSerialize, BorshDeserialize, Serialize, Deserialize, UniversalWallet, Arbitrary,
)]
#[sov_wallet(anonymize_tuple)]
pub struct AnonymizedTupleStruct(i16, Option<u8>);

/// Tuple structs nested inside each other and inside containers.
#[derive(
    Debug, BorshSerialize, BorshDeserialize, Serialize, Deserialize, UniversalWallet, Arbitrary,
)]
pub struct NestedTupleStruct(
    NewtypeStruct,
    Option<NamedTupleStruct>,
    Vec<AnonymizedTupleStruct>,
);

#[derive(
    Debug, BorshSerialize, BorshDeserialize, Serialize, Deserialize, UniversalWallet, Arbitrary,
)]
pub enum TupleStructInput {
    Named(NamedTupleStruct),
    Newtype(NewtypeStruct),
    Anonymized(AnonymizedTupleStruct),
    Nested(NestedTupleStruct),
}

#[derive(
    Debug, BorshSerialize, BorshDeserialize, Serialize, Deserialize, UniversalWallet, Arbitrary,
)]
pub enum FuzzInput {
    Bool(bool),
    String(ArbitrarySafeString),
    ByteVec(ByteVecInput),
    Vec(Vec<(i8, u16)>),
    ByteArray(ByteArrayInput),
    Array([i16; 5]),
    // TODO: Map
    Number(NumberInput),
    InlineStruct {
        field: u32,
        name: ArbitrarySafeString,
    },
    MultiTuple(i8, Option<u8>),
    TupleStruct(TupleStructInput),
    SkippedField(SkippedField),
    Complex(ComplexStruct),
    Null(()),
}

#[cfg(test)]
mod tests {
    use sov_universal_wallet::schema::Schema;
    use sov_universal_wallet::ty::Ty;

    use super::*;

    /// The type names recorded on the tuple types of `FuzzInput`'s schema.
    fn recorded_tuple_type_names() -> Vec<String> {
        Schema::of_single_type::<FuzzInput>()
            .unwrap()
            .types()
            .iter()
            .filter_map(|ty| match ty {
                Ty::Tuple(tuple) => tuple.type_name.clone(),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn fuzz_input_schema_records_tuple_struct_names() {
        let names = recorded_tuple_type_names();
        for expected in ["NamedTupleStruct", "NewtypeStruct", "NestedTupleStruct"] {
            assert!(
                names.iter().any(|name| name == expected),
                "FuzzInput's schema should contain a tuple type named {expected}, found {names:?}"
            );
        }
    }

    #[test]
    fn fuzz_input_schema_omits_anonymized_tuple_struct_name() {
        let names = recorded_tuple_type_names();
        assert!(
            !names.iter().any(|name| name == "AnonymizedTupleStruct"),
            "A tuple struct annotated with anonymize_tuple should not record its name, found {names:?}"
        );
    }
}
