use sov_modules_api::macros::config_value;

// Actual value is 2 bytes
const NAMESPACE: [u8; 32] = config_value!("HEX_SHORT");

fn main() {}
