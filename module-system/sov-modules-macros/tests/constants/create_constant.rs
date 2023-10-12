use sov_modules_api::macros::config_constant;
#[config_constant]
pub const TEST_U32: u32 = 0;

#[config_constant]
pub const TEST_ARRAY_OF_U8: [u8; 32] = [0; 32];

#[config_constant]
pub const TEST_NESTED_ARRAY: [[u8; 3]; 2] = [[0; 3]; 2];

#[config_constant]
pub const TEST_BOOL: bool = false;

#[config_constant]
pub const TEST_STRING: &str = "Some String";

fn main() {
    assert_eq!(TEST_U32, 42);
    assert_eq!(TEST_ARRAY_OF_U8, [11; 32]);
    assert_eq!(TEST_NESTED_ARRAY, [[7; 3]; 2]);
    assert_eq!(TEST_BOOL, true);
    assert_eq!(TEST_STRING, "Some Other String");
}
