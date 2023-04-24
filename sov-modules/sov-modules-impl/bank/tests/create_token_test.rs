use bank::call::CallMessage;
use bank::query::{BalanceResponse, QueryMessage};
use bank::{create_token_address, Bank};
use sov_modules_api::{Context, Module, ModuleInfo};
use sov_state::{ProverStorage, WorkingSet};

mod helpers;

use helpers::*;

#[test]
fn initial_and_deployed_token() {
    let bank_config = create_bank_config_with_token(1, 100);
    let mut working_set = WorkingSet::new(ProverStorage::temporary());
    let bank = Bank::new();
    bank.genesis(&bank_config, &mut working_set).unwrap();

    let sender_address = generate_address("sender");
    let sender_context = C::new(sender_address.clone());
    let minter_address = generate_address("minter");
    let initial_balance = 500;
    let token_name = "Token1".to_owned();
    let salt = 1;
    let token_address = create_token_address::<C>(&token_name, sender_address.as_ref(), salt);
    let create_token_message = CallMessage::CreateToken::<C> {
        salt,
        token_name,
        initial_balance,
        minter_address: minter_address.clone(),
    };

    let create_token_response = bank
        .call(create_token_message, &sender_context, &mut working_set)
        .expect("Failed to create token");

    assert!(create_token_response.events.is_empty());

    let query = QueryMessage::GetBalance {
        user_address: sender_address,
        token_address: token_address.clone(),
    };

    let sender_balance = query_and_deserialize::<BalanceResponse>(&bank, query, &mut working_set);
    assert!(sender_balance.amount.is_none());

    let query = QueryMessage::GetBalance {
        user_address: minter_address,
        token_address,
    };

    let minter_balance = query_and_deserialize::<BalanceResponse>(&bank, query, &mut working_set);

    assert_eq!(Some(initial_balance), minter_balance.amount);
}

#[test]
/// Currently integer overflow happens on bank genesis
fn overflow_max_supply() {
    let bank = Bank::<C>::new();
    let mut working_set = WorkingSet::new(ProverStorage::temporary());

    let bank_config = create_bank_config_with_token(2, u64::MAX - 2);

    let genesis_result = bank.genesis(&bank_config, &mut working_set);
    assert!(genesis_result.is_err());

    assert_eq!(
        "Total supply overflow",
        genesis_result.unwrap_err().to_string()
    );
}
