use sov_bank::{BankConfig, TokenConfig};
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::utils::generate_address as gen_address_generic;
use sov_modules_api::Address;

pub type C = DefaultContext;

// This code is not actually dead; rustc treats each test file as a separate crate
// so this code looks unused during some of the compilations.
#[allow(dead_code)]
pub fn generate_address(name: &str) -> Address {
    gen_address_generic::<C>(name)
}

#[allow(dead_code)]
pub fn create_bank_config_with_token(
    addresses_count: usize,
    initial_balance: u64,
) -> BankConfig<C> {
    let address_and_balances = (0..addresses_count)
        .map(|i| {
            let key = format!("key_{}", i);
            let addr = generate_address(&key);
            (addr, initial_balance)
        })
        .collect();

    let token_config = TokenConfig {
        token_name: "InitialToken".to_owned(),
        address_and_balances,
        authorized_minters: vec![],
        salt: 5,
    };

    BankConfig {
        tokens: vec![token_config],
    }
}
