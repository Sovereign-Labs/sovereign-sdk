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
}
