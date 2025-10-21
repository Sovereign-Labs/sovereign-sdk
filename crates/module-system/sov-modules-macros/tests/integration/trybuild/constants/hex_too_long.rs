use sov_modules_api::macros::config_value;

// Actual value is 32 bytes
const NAMESPACE: [u8; 10] = config_value!("HEX_LONG");

fn main() {}
