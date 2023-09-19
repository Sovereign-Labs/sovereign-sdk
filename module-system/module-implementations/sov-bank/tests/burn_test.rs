use helpers::{generate_address, C};
use sov_bank::{
    get_genesis_token_address, get_token_address, Bank, BankConfig, CallMessage, Coins,
    TotalSupplyResponse,
};
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::{Address, Context, Error, Module, WorkingSet};
use sov_state::{DefaultStorageSpec, ProverStorage};

use crate::helpers::create_bank_config_with_token;

mod helpers;

pub type Storage = ProverStorage<DefaultStorageSpec>;

#[test]
fn burn_deployed_tokens() {
    let bank = Bank::<C>::default();
    let tmpdir = tempfile::tempdir().unwrap();
    let mut working_set = WorkingSet::new(ProverStorage::with_path(tmpdir.path()).unwrap());
    let empty_bank_config = BankConfig::<C> { tokens: vec![] };
    bank.genesis(&empty_bank_config, &mut working_set).unwrap();

    let sender_address = generate_address("just_sender");
    let sender_context = C::new(sender_address);
    let minter_address = generate_address("minter");
    let minter_context = C::new(minter_address);

    let salt = 0;
    let token_name = "Token1".to_owned();
    let initial_balance = 100;
    let token_address = get_token_address::<C>(&token_name, minter_address.as_ref(), salt);

    // ---
    // Deploying token
    let mint_message = CallMessage::CreateToken {
        salt,
        token_name,
        initial_balance,
        minter_address,
        authorized_minters: vec![minter_address],
    };
    bank.call(mint_message, &minter_context, &mut working_set)
        .expect("Failed to mint token");
    // No events at the moment. If there are, needs to be checked
    assert!(working_set.events().is_empty());

    let query_total_supply = |working_set: &mut WorkingSet<DefaultContext>| -> Option<u64> {
        let total_supply: TotalSupplyResponse = bank.supply_of(token_address, working_set).unwrap();
        total_supply.amount
    };

    let query_user_balance =
        |user_address: Address, working_set: &mut WorkingSet<DefaultContext>| -> Option<u64> {
            bank.get_balance_of(user_address, token_address, working_set)
        };

    let previous_total_supply = query_total_supply(&mut working_set);
    assert_eq!(Some(initial_balance), previous_total_supply);

    // -----
    // Burn
    let burn_amount = 10;
    let burn_message = CallMessage::Burn {
        coins: Coins {
            amount: burn_amount,
            token_address,
        },
    };

    bank.call(burn_message.clone(), &minter_context, &mut working_set)
        .expect("Failed to burn token");
    assert!(working_set.events().is_empty());

    let current_total_supply = query_total_supply(&mut working_set);
    assert_eq!(Some(initial_balance - burn_amount), current_total_supply);
    let minter_balance = query_user_balance(minter_address, &mut working_set);
    assert_eq!(Some(initial_balance - burn_amount), minter_balance);

    let previous_total_supply = current_total_supply;
    // ---
    // Burn by another user, who doesn't have tokens at all
    let failed_to_burn = bank.call(burn_message, &sender_context, &mut working_set);
    assert!(failed_to_burn.is_err());
    let Error::ModuleError(err) = failed_to_burn.err().unwrap();
    let mut chain = err.chain();
    let message_1 = chain.next().unwrap().to_string();
    let message_2 = chain.next().unwrap().to_string();
    assert!(chain.next().is_none());
    assert_eq!(
        format!(
            "Failed to burn coins(token_address={} amount={}) from owner {}",
            token_address, burn_amount, sender_address
        ),
        message_1
    );
    let expected_error_part = format!(
        "Value not found for prefix: \"sov_bank/Bank/tokens/{}\" and: storage key",
        token_address
    );
    assert!(message_2.starts_with(&expected_error_part));

    let current_total_supply = query_total_supply(&mut working_set);
    assert_eq!(previous_total_supply, current_total_supply);
    let sender_balance = query_user_balance(sender_address, &mut working_set);
    assert_eq!(None, sender_balance);

    // ---
    // Allow burning zero tokens
    let burn_zero_message = CallMessage::Burn {
        coins: Coins {
            amount: 0,
            token_address,
        },
    };

    bank.call(burn_zero_message, &minter_context, &mut working_set)
        .expect("Failed to burn token");
    assert!(working_set.events().is_empty());
    let minter_balance_after = query_user_balance(minter_address, &mut working_set);
    assert_eq!(minter_balance, minter_balance_after);

    // ---
    // Burn more than available
    let burn_message = CallMessage::Burn {
        coins: Coins {
            amount: initial_balance + 10,
            token_address,
        },
    };

    let failed_to_burn = bank.call(burn_message, &minter_context, &mut working_set);
    assert!(failed_to_burn.is_err());
    let Error::ModuleError(err) = failed_to_burn.err().unwrap();
    let mut chain = err.chain();
    let message_1 = chain.next().unwrap().to_string();
    let message_2 = chain.next().unwrap().to_string();
    assert!(chain.next().is_none());
    assert_eq!(
        format!(
            "Failed to burn coins(token_address={} amount={}) from owner {}",
            token_address,
            initial_balance + 10,
            minter_address
        ),
        message_1
    );
    assert_eq!(
        format!("Insufficient funds for {}", minter_address),
        message_2
    );

    // ---
    // Try to burn non existing token
    let token_address = get_token_address::<C>("NotRealToken2", minter_address.as_ref(), salt);
    let burn_message = CallMessage::Burn {
        coins: Coins {
            amount: 1,
            token_address,
        },
    };

    let failed_to_burn = bank.call(burn_message, &minter_context, &mut working_set);
    assert!(failed_to_burn.is_err());
    let Error::ModuleError(err) = failed_to_burn.err().unwrap();
    let mut chain = err.chain();
    let message_1 = chain.next().unwrap().to_string();
    let message_2 = chain.next().unwrap().to_string();
    assert!(chain.next().is_none());
    assert_eq!(
        format!(
            "Failed to burn coins(token_address={} amount={}) from owner {}",
            token_address, 1, minter_address
        ),
        message_1
    );
    // Note, no token address in root cause message.
    let expected_error_part =
        "Value not found for prefix: \"sov_bank/Bank/tokens/\" and: storage key";
    assert!(message_2.starts_with(expected_error_part));
}

#[test]
fn burn_initial_tokens() {
    let initial_balance = 100;
    let bank_config = create_bank_config_with_token(1, initial_balance);
    let tmpdir = tempfile::tempdir().unwrap();
    let mut working_set = WorkingSet::new(ProverStorage::with_path(tmpdir.path()).unwrap());
    let bank = Bank::default();
    bank.genesis(&bank_config, &mut working_set).unwrap();

    let token_address = get_genesis_token_address::<C>(
        &bank_config.tokens[0].token_name,
        bank_config.tokens[0].salt,
    );
    let sender_address = bank_config.tokens[0].address_and_balances[0].0;

    let query_user_balance =
        |user_address: Address, working_set: &mut WorkingSet<DefaultContext>| -> Option<u64> {
            bank.get_balance_of(user_address, token_address, working_set)
        };

    let balance_before = query_user_balance(sender_address, &mut working_set);
    assert_eq!(Some(initial_balance), balance_before);

    let burn_amount = 10;
    let burn_message = CallMessage::Burn {
        coins: Coins {
            amount: burn_amount,
            token_address,
        },
    };

    let context = C::new(sender_address);
    bank.call(burn_message, &context, &mut working_set)
        .expect("Failed to burn token");
    assert!(working_set.events().is_empty());

    let balance_after = query_user_balance(sender_address, &mut working_set);
    assert_eq!(Some(initial_balance - burn_amount), balance_after);

    // Assume that the rest of edge cases are similar to deployed tokens
}
