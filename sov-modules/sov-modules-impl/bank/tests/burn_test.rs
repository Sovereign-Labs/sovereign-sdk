use borsh::BorshSerialize;

use bank::call::CallMessage;
use bank::query::{BalanceResponse, QueryMessage, TotalSupplyResponse};
use bank::{create_token_address, Bank, BankConfig, Coins};
use helpers::*;
use sov_modules_api::{Address, Context, Module, ModuleInfo};
use sov_state::{ProverStorage, WorkingSet};

mod helpers;

#[test]
fn burn_tokens() {
    let bank = Bank::<C>::new();
    let mut working_set = WorkingSet::new(ProverStorage::temporary());
    let empty_bank_config = BankConfig::<C> { tokens: vec![] };
    bank.genesis(&empty_bank_config, &mut working_set).unwrap();

    let sender_address = generate_address("just_sender");
    let sender_context = C::new(sender_address.clone());
    let minter_address = generate_address("minter");
    let minter_context = C::new(minter_address.clone());

    let salt = 0;
    let token_name = "Token1".to_owned();
    let initial_balance = 100;
    let token_address = create_token_address::<C>(&token_name, minter_address.as_ref(), salt);

    // ---
    // Deploying token
    let mint_message = CallMessage::CreateToken {
        salt,
        token_name,
        initial_balance,
        minter_address: minter_address.clone(),
    };
    let minted = bank
        .call(mint_message, &minter_context, &mut working_set)
        .expect("Failed to mint token");
    // No events at the moment. If there are, needs to be checked
    assert!(minted.events.is_empty());

    let query_total_supply = |working_set: &mut WorkingSet<Storage>| -> Option<u64> {
        let query = QueryMessage::GetTotalSupply {
            token_address: token_address.clone(),
        };
        let total_supply: TotalSupplyResponse = query_and_deserialize(&bank, query, working_set);
        total_supply.amount
    };

    let query_user_balance =
        |user_address: Address, working_set: &mut WorkingSet<Storage>| -> Option<u64> {
            let query = QueryMessage::GetBalance {
                user_address,
                token_address: token_address.clone(),
            };

            let balance: BalanceResponse = query_and_deserialize(&bank, query, working_set);

            balance.amount
        };

    let current_total_supply = query_total_supply(&mut working_set);
    assert_eq!(Some(initial_balance), current_total_supply);

    // -----
    // Burn
    let burn_amount = 10;
    let burn_message = CallMessage::Burn {
        coins: Coins {
            amount: burn_amount,
            token_address: token_address.clone(),
        },
    };

    let burned = bank
        .call(burn_message.clone(), &minter_context, &mut working_set)
        .expect("Failed to burn token");
    assert!(burned.events.is_empty());

    let current_total_supply = query_total_supply(&mut working_set);
    // Total supply does not change
    assert_eq!(Some(initial_balance), current_total_supply);
    let minter_balance = query_user_balance(minter_address.clone(), &mut working_set);
    assert_eq!(Some(initial_balance - burn_amount), minter_balance);

    // ---
    // Burn by another user, who doesn't have tokens at all
    let failed_to_burn = bank.call(burn_message, &sender_context, &mut working_set);
    assert!(failed_to_burn.is_err());
    let expected_error = format!(
        "Value not found for prefix: 0x{}",
        hex::encode(token_address.try_to_vec().unwrap())
    );
    assert!(failed_to_burn
        .err()
        .unwrap()
        .to_string()
        .contains(&expected_error));
    let current_total_supply = query_total_supply(&mut working_set);
    assert_eq!(Some(initial_balance), current_total_supply);
    let sender_balance = query_user_balance(sender_address, &mut working_set);
    assert_eq!(None, sender_balance);

    // ---
    // Allow burning zero tokens
    let burn_zero_message = CallMessage::Burn {
        coins: Coins {
            amount: 0,
            token_address: token_address.clone(),
        },
    };

    let burned_zero = bank
        .call(burn_zero_message, &minter_context, &mut working_set)
        .expect("Failed to burn token");
    assert!(burned_zero.events.is_empty());
    let minter_balance_after = query_user_balance(minter_address.clone(), &mut working_set);
    assert_eq!(minter_balance, minter_balance_after);

    // ---
    // Try to burn non existing token
    let token_address = create_token_address::<C>("NotRealToken2", minter_address.as_ref(), salt);
    let burn_message = CallMessage::Burn {
        coins: Coins {
            amount: 1,
            token_address,
        },
    };

    let failed_to_burn = bank.call(burn_message, &minter_context, &mut working_set);
    assert!(failed_to_burn.is_err());
    assert!(failed_to_burn
        .err()
        .unwrap()
        .to_string()
        .contains("Value not found for prefix: \"bank/Bank/tokens/\" and: storage key"));

    // ---
    // Burn more than available
}
