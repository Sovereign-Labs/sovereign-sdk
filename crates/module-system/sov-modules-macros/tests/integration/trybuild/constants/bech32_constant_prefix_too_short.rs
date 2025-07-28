use sov_modules_api::impl_hash32_type;
use sov_modules_api::macros::config_value;

impl_hash32_type!(MyTokenId, MyTokenBech, "token_with_long_prefix");

const TOKEN: MyTokenId = config_value!("CONST_TOKEN_ID");

fn main() {}
