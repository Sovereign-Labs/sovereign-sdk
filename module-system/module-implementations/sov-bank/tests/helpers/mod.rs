use sov_bank::{BankConfig, TokenConfig};
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::{Address, Hasher, Spec};

pub type C = DefaultContext;

pub fn generate_address(key: &str) -> <C as Spec>::Address {
    let hash = <C as Spec>::Hasher::hash(key.as_bytes());
    Address::from(hash)
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
    };

    BankConfig {
        tokens: vec![token_config],
    }
}
