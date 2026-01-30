#![allow(dead_code, non_upper_case_globals)]
use std::env;

use sov_modules_api::impl_hash32_type;
use sov_modules_api::macros::config_value;

impl_hash32_type!(MyTokenId, MyTokenBech, "token_");

// Make sure that non-overridable constants compile. If compilation pass,
// there's not much need to do anything else.
// -----------------------------------------------------------------------------
const CONST_TOKEN_ID: MyTokenId = config_value!("CONST_TOKEN_ID");
const CONST_I64_MAX: i64 = config_value!("CONST_I64_MAX");
const CONST_BOOL: bool = config_value!("CONST_BOOL");
const CONST_STRING: &str = config_value!("CONST_STRING");
const CONST_MATRIX_2x3: [[u8; 3]; 2] = config_value!("CONST_MATRIX_2x3");
const CONST_MATRIX_2x3_I32: [[i32; 3]; 2] = config_value!("CONST_MATRIX_2x3");

// Test hex constants (const and non-const)
const CONST_HEX_ARRAY: [u8; 5] = config_value!("CONST_HEX_ARRAY");
const HEX_SHORT: [u8; 2] = config_value!("HEX_SHORT");
const HEX_MEDIUM: [u8; 10] = config_value!("HEX_MEDIUM");
const HEX_LONG: [u8; 32] = config_value!("HEX_LONG");
const HEX_NO_PREFIX: [u8; 4] = config_value!("HEX_NO_PREFIX");
const HEX_EMPTY: [u8; 0] = config_value!("HEX_EMPTY");

// Test byte_string constants (const and non-const)
const CONST_BYTE_STRING: [u8; 8] = config_value!("CONST_BYTE_STRING");
const BYTE_STRING_ASCII: [u8; 10] = config_value!("BYTE_STRING_ASCII");
const BYTE_STRING_SHORT: [u8; 5] = config_value!("BYTE_STRING_SHORT");
const BYTE_STRING_UTF8: [u8; 10] = config_value!("BYTE_STRING_UTF8");
const BYTE_STRING_EMOJI: [u8; 10] = config_value!("BYTE_STRING_EMOJI");
const BYTE_STRING_MIXED: [u8; 10] = config_value!("BYTE_STRING_MIXED");
const BYTE_STRING_EMPTY: [u8; 0] = config_value!("BYTE_STRING_EMPTY");
const BYTE_STRING_SINGLE: [u8; 1] = config_value!("BYTE_STRING_SINGLE");

// Now, let's make sure that overridable constants compile AND that env. var.
// reading logic works.
// -----------------------------------------------------------------------------

fn token_id() -> MyTokenId {
    config_value!("TOKEN_ID")
}

fn i64_min() -> i64 {
    config_value!("I64_MIN")
}

fn non_const_bool() -> bool {
    config_value!("NON_CONST_BOOL")
}

fn non_const_string() -> &'static str {
    config_value!("NON_CONST_STRING")
}

fn matrix_2x3() -> [[u8; 3]; 2] {
    config_value!("MATRIX_2x3")
}

fn empty_array() -> [u8; 0] {
    config_value!("EMPTY_ARRAY")
}

fn array_of_bech32() -> [MyTokenId; 2] {
    config_value!("ARRAY_OF_BECH32")
}

fn array_of_u8() -> [u8; 32] {
    config_value!("ARRAY_OF_U8")
}

fn chain_id_u128() -> u128 {
    config_value!("CHAIN_ID")
}

fn hex_medium() -> [u8; 10] {
    config_value!("HEX_MEDIUM")
}

fn byte_string_hello() -> [u8; 5] {
    config_value!("BYTE_STRING_SHORT")
}

fn main() {
    env::set_var(
        "SOV_TEST_CONST_OVERRIDE_TOKEN_ID",
        "token_1rwrh8gn2py0dl4vv65twgctmlwck6esm2as9dftumcw89kqqn3nqrduss6",
    );
    assert_eq!(
        token_id().to_string(),
        "token_1rwrh8gn2py0dl4vv65twgctmlwck6esm2as9dftumcw89kqqn3nqrduss6"
    );

    env::set_var("SOV_TEST_CONST_OVERRIDE_I64_MIN", "2");
    assert_eq!(i64_min(), 2);

    env::set_var("SOV_TEST_CONST_OVERRIDE_NON_CONST_BOOL", "false");
    assert_eq!(non_const_bool(), false);

    env::set_var("SOV_TEST_CONST_OVERRIDE_NON_CONST_STRING", "spam");
    assert_eq!(non_const_string(), "spam");

    env::set_var(
        "SOV_TEST_CONST_OVERRIDE_MATRIX_2x3",
        "[[0, 0, 0], [1, 1, 1]]",
    );
    assert_eq!(matrix_2x3(), [[0, 0, 0], [1, 1, 1]]);

    env::set_var("SOV_TEST_CONST_OVERRIDE_EMPTY_ARRAY", "[]");
    assert_eq!(empty_array(), [0u8; 0]);

    env::set_var(
        "SOV_TEST_CONST_OVERRIDE_ARRAY_OF_BECH32",
        r#"["token_1rwrh8gn2py0dl4vv65twgctmlwck6esm2as9dftumcw89kqqn3nqrduss6", "token_1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqnfxkwm"]"#,
    );
    assert_eq!(
        array_of_bech32()[0].to_string(),
        "token_1rwrh8gn2py0dl4vv65twgctmlwck6esm2as9dftumcw89kqqn3nqrduss6"
    );
    assert_eq!(
        array_of_bech32()[1].to_string(),
        "token_1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqnfxkwm"
    );

    env::set_var(
        "SOV_TEST_CONST_OVERRIDE_ARRAY_OF_U8",
        "[1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1]",
    );
    assert_eq!(array_of_u8(), [1; 32]);

    env::set_var("SOV_TEST_CONST_OVERRIDE_CHAIN_ID", "0");
    assert_eq!(chain_id_u128(), 0);

    // Test hex constants
    assert_eq!(CONST_HEX_ARRAY, [1u8, 2, 3, 4, 5]);
    assert_eq!(HEX_SHORT, [0xaa, 0xbb]);
    let hex_medium_value: [u8; 10] = [0x73, 0x6f, 0x76, 0x2d, 0x74, 0x65, 0x73, 0x74, 0x2d, 0x70];
    assert_eq!(HEX_MEDIUM, hex_medium_value,);
    assert_eq!(hex_medium(), hex_medium_value);
    let empty_bytes: [u8; 0] = [];
    assert_eq!(HEX_NO_PREFIX, [0xaa, 0xbb, 0xcc, 0xdd]);
    assert_eq!(HEX_EMPTY, empty_bytes);
    assert_eq!(
        HEX_LONG,
        [
            1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24,
            25, 26, 27, 28, 29, 30, 31, 32
        ]
    );

    // Test byte_string constants
    assert_eq!(CONST_BYTE_STRING, *b"constant");
    assert_eq!(BYTE_STRING_ASCII, *b"sov-test-b");
    assert_eq!(BYTE_STRING_SHORT, *b"hello");
    assert_eq!(byte_string_hello(), *b"hello");
    assert_eq!(BYTE_STRING_SINGLE, *b"a");
    assert_eq!(BYTE_STRING_EMPTY, empty_bytes);

    // UTF-8 tests (all 10 bytes)
    assert_eq!(BYTE_STRING_UTF8, "абвгд".as_bytes());
    assert_eq!(BYTE_STRING_EMOJI, "🎉hello!".as_bytes());
    assert_eq!(BYTE_STRING_MIXED, "Hi!мир!".as_bytes());
}
