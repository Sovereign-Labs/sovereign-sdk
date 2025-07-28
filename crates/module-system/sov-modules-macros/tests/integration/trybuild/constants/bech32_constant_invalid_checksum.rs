use sov_modules_api::impl_hash32_type;
use sov_modules_api::macros::config_value;

impl_hash32_type!(MyTokenId, MyTokenBech, "tok");

const TOKEN_ID: MyTokenId = config_value!("CONST_TOKEN_ID_INVALID_CHECKSUM");

fn main() {}
