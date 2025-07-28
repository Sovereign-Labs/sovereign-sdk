use sov_bank::{config_gas_token_id, Coins};
use sov_modules_api::Amount;

pub(crate) fn gas_coins(amount: Amount) -> Coins {
    Coins {
        amount,
        token_id: config_gas_token_id(),
    }
}
