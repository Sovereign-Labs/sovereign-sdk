use sov_modules_api::impl_hash32_type;
use sov_modules_api::macros::config_value;

impl_hash32_type!(MyTokenId, MyTokenBech, "tok");

const TOKEN: MyTokenId = config_value!("GAS_DIMENSIONS");

fn main() {}
