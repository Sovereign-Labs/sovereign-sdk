use bank::call::CallMessage;
use bank::{Bank, BankConfig, TokenConfig};
use sov_modules_api::{Context, Module, ModuleInfo};
use sov_state::{ProverStorage, WorkingSet};

mod helpers;

use helpers::*;

#[test]
fn initial_and_deployed_token() {
    let mut test_bank = create_test_bank_with_token(2, 100);

    let sender_address = generate_address("sender");
    let sender_context = C::new(sender_address);
    let minter_address = generate_address("minter");
    let initial_balance = 500;
    let create_token_message = CallMessage::CreateToken::<C> {
        salt: 1,
        token_name: "Token1".to_owned(),
        initial_balance,
        minter_address: minter_address.clone(),
    };

    let create_token_response = test_bank
        .bank
        .call(
            create_token_message,
            &sender_context,
            &mut test_bank.working_set,
        )
        .expect("Failed to create token");

    assert!(create_token_response.events.is_empty());
}

#[test]
/// Currently integer overflow happens on bank genesis
fn integer_overflow() {
    let bank = Bank::<C>::new();
    let mut working_set = WorkingSet::new(ProverStorage::temporary());

    let bank_config = BankConfig {
        tokens: vec![TokenConfig {
            token_name: "Token1".to_string(),
            address_and_balances: vec![
                (generate_address("user1"), u64::MAX - 2),
                (generate_address("user2"), u64::MAX - 2),
            ],
        }],
    };

    let genesis_result = bank.genesis(&bank_config, &mut working_set);
    assert!(genesis_result.is_err());

    assert_eq!(
        "Total supply overflow",
        genesis_result.unwrap_err().to_string()
    );
}
