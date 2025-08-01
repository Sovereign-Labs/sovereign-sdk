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

#[derive(
    Debug, BorshSerialize, BorshDeserialize, Serialize, Deserialize, UniversalWallet, Arbitrary,
)]
pub enum ByteVecInput {
    Hex(#[sov_wallet(display(hex))] Vec<u8>),
    Base58(#[sov_wallet(display(base58))] Vec<u8>),
    Decimal(#[sov_wallet(display(decimal))] Vec<u8>),
}

#[derive(
    Debug, BorshSerialize, BorshDeserialize, Serialize, Deserialize, UniversalWallet, Arbitrary,
)]
pub enum ByteArrayInput {
    Hex(#[sov_wallet(display(hex))] [u8; 32]),
    Base58(#[sov_wallet(display(base58))] [u8; 32]),
    Decimal(#[sov_wallet(display(decimal))] [u8; 32]),
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
    bulk_tuple: (i32, i32, u64, I128, ArbitrarySafeString, Option<bool>),
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
    SkippedField(SkippedField),
    Complex(ComplexStruct),
    Null(()),
}
