mod helpers;

use helpers::*;
use sov_bank::call::CallMessage;
use sov_bank::genesis::{DEPLOYER, SALT};
use sov_bank::query::TotalSupplyResponse;
use sov_bank::{create_token_address, Bank, BankConfig, Coins};
use sov_modules_api::{Address, Context, Module};
use sov_state::{DefaultStorageSpec, ProverStorage, WorkingSet};

pub type Storage = ProverStorage<DefaultStorageSpec>;

#[test]
fn transfer_initial_token() {
    let initial_balance = 100;
    let transfer_amount = 10;
    let bank_config = create_bank_config_with_token(3, initial_balance);
    let tmpdir = tempfile::tempdir().unwrap();
    let mut working_set = WorkingSet::new(ProverStorage::with_path(tmpdir.path()).unwrap());
    let bank = Bank::default();
    bank.genesis(&bank_config, &mut working_set).unwrap();

    let token_address =
        create_token_address::<C>(&bank_config.tokens[0].token_name, &DEPLOYER, SALT);
    let sender_address = bank_config.tokens[0].address_and_balances[0].0.clone();
    let receiver_address = bank_config.tokens[0].address_and_balances[1].0.clone();
    assert_ne!(sender_address, receiver_address);

    // Preparation
    let query_user_balance =
        |user_address: Address, working_set: &mut WorkingSet<Storage>| -> Option<u64> {
            bank.get_balance_of(user_address, token_address.clone(), working_set)
        };

    let query_total_supply = |working_set: &mut WorkingSet<Storage>| -> Option<u64> {
        let total_supply: TotalSupplyResponse = bank.supply_of(token_address.clone(), working_set);
        total_supply.amount
    };

    let sender_balance_before = query_user_balance(sender_address.clone(), &mut working_set);
    let receiver_balance_before = query_user_balance(receiver_address.clone(), &mut working_set);
    let total_supply_before = query_total_supply(&mut working_set);
    assert!(total_supply_before.is_some());

    assert_eq!(Some(initial_balance), sender_balance_before);
    assert_eq!(sender_balance_before, receiver_balance_before);
    let sender_context = C::new(sender_address.clone());

    // Transfer happy test
    {
        let transfer_message = CallMessage::Transfer {
            to: receiver_address.clone(),
            coins: Coins {
                amount: transfer_amount,
                token_address: token_address.clone(),
            },
        };

        bank.call(transfer_message, &sender_context, &mut working_set)
            .expect("Transfer call failed");
        assert!(working_set.events().is_empty());

        let sender_balance_after = query_user_balance(sender_address.clone(), &mut working_set);
        let receiver_balance_after = query_user_balance(receiver_address.clone(), &mut working_set);

        assert_eq!(
            Some(initial_balance - transfer_amount),
            sender_balance_after
        );
        assert_eq!(
            Some(initial_balance + transfer_amount),
            receiver_balance_after
        );
        let total_supply_after = query_total_supply(&mut working_set);
        assert_eq!(total_supply_before, total_supply_after);
    }

    // Not enough balance
    {
        let transfer_message = CallMessage::Transfer {
            to: receiver_address.clone(),
            coins: Coins {
                amount: initial_balance + 1,
                token_address: token_address.clone(),
            },
        };

        let result = bank.call(transfer_message, &sender_context, &mut working_set);
        assert!(result.is_err());
        let error = result.err().unwrap();
        assert_eq!(
            "Insufficient funds for sov1h5567we4l0ne5vyrkvqd6jq5qp2cs7sa780vut0vrwr8pytwrzess8mu2s",
            error.to_string()
        );
    }

    // Non existent token
    {
        let salt = 0;
        let token_name = "NonExistingToken".to_owned();
        let token_address = create_token_address::<C>(&token_name, sender_address.as_ref(), salt);

        let transfer_message = CallMessage::Transfer {
            to: receiver_address.clone(),
            coins: Coins {
                amount: 1,
                token_address: token_address.clone(),
            },
        };

        let result = bank.call(transfer_message, &sender_context, &mut working_set);
        assert!(result.is_err());
        let error = result.err().unwrap();
        assert!(error
            .to_string()
            .contains("Value not found for prefix: \"sov_bank/Bank/tokens/\" and: storage key"))
    }

    // Sender does not exist
    {
        let unknown_sender = generate_address("non_existing_sender");
        let unknown_sender_context = C::new(unknown_sender.clone());

        let sender_balance = query_user_balance(unknown_sender.clone(), &mut working_set);
        assert!(sender_balance.is_none());

        let receiver_balance_before =
            query_user_balance(receiver_address.clone(), &mut working_set);

        let transfer_message = CallMessage::Transfer {
            to: receiver_address.clone(),
            coins: Coins {
                amount: 1,
                token_address: token_address.clone(),
            },
        };

        let result = bank.call(transfer_message, &unknown_sender_context, &mut working_set);
        assert!(result.is_err());
        let error = result.err().unwrap();

        let expected_message_part = format!(
            "Value not found for prefix: \"sov_bank/Bank/tokens/{}\" and: storage key",
            token_address
        );
        let actual_message = error.to_string();
        assert!(actual_message.contains(&expected_message_part));

        let receiver_balance_after = query_user_balance(receiver_address, &mut working_set);
        assert_eq!(receiver_balance_before, receiver_balance_after);
    }

    // Receiver does not exist
    {
        let unknown_receiver = generate_address("non_existing_receiver");

        let receiver_balance_before =
            query_user_balance(unknown_receiver.clone(), &mut working_set);
        assert!(receiver_balance_before.is_none());

        let transfer_message = CallMessage::Transfer {
            to: unknown_receiver.clone(),
            coins: Coins {
                amount: 1,
                token_address: token_address.clone(),
            },
        };

        bank.call(transfer_message, &sender_context, &mut working_set)
            .expect("Transfer call failed");
        assert!(working_set.events().is_empty());

        let receiver_balance_after = query_user_balance(unknown_receiver, &mut working_set);
        assert_eq!(Some(1), receiver_balance_after)
    }

    // Sender equals receiver
    {
        let total_supply_before = query_total_supply(&mut working_set);
        let sender_balance_before = query_user_balance(sender_address.clone(), &mut working_set);
        assert!(sender_balance_before.is_some());

        let transfer_message = CallMessage::Transfer {
            to: sender_address.clone(),
            coins: Coins {
                amount: 1,
                token_address: token_address.clone(),
            },
        };
        bank.call(transfer_message, &sender_context, &mut working_set)
            .expect("Transfer call failed");
        assert!(working_set.events().is_empty());

        let sender_balance_after = query_user_balance(sender_address, &mut working_set);
        assert_eq!(sender_balance_before, sender_balance_after);
        let total_supply_after = query_total_supply(&mut working_set);
        assert_eq!(total_supply_after, total_supply_before);
    }
}

#[test]
fn transfer_deployed_token() {
    let bank = Bank::<C>::default();
    let tmpdir = tempfile::tempdir().unwrap();
    let mut working_set = WorkingSet::new(ProverStorage::with_path(tmpdir.path()).unwrap());
    let empty_bank_config = BankConfig::<C> { tokens: vec![] };
    bank.genesis(&empty_bank_config, &mut working_set).unwrap();

    let sender_address = generate_address("just_sender");
    let receiver_address = generate_address("just_receiver");

    let salt = 10;
    let token_name = "Token1".to_owned();
    let initial_balance = 1000;
    let token_address = create_token_address::<C>(&token_name, sender_address.as_ref(), salt);

    assert_ne!(sender_address, receiver_address);

    // Preparation
    let query_user_balance =
        |user_address: Address, working_set: &mut WorkingSet<Storage>| -> Option<u64> {
            bank.get_balance_of(user_address, token_address.clone(), working_set)
        };

    let query_total_supply = |working_set: &mut WorkingSet<Storage>| -> Option<u64> {
        let total_supply: TotalSupplyResponse = bank.supply_of(token_address.clone(), working_set);
        total_supply.amount
    };

    let sender_balance_before = query_user_balance(sender_address.clone(), &mut working_set);
    let receiver_balance_before = query_user_balance(receiver_address.clone(), &mut working_set);
    let total_supply_before = query_total_supply(&mut working_set);
    assert!(total_supply_before.is_none());

    assert!(sender_balance_before.is_none());
    assert!(receiver_balance_before.is_none());
    let sender_context = C::new(sender_address.clone());

    let mint_message = CallMessage::CreateToken {
        salt,
        token_name,
        initial_balance,
        minter_address: sender_address.clone(),
        authorized_minters: vec![sender_address.clone()],
    };
    bank.call(mint_message, &sender_context, &mut working_set)
        .expect("Failed to mint token");
    // No events at the moment. If there are, needs to be checked
    assert!(working_set.events().is_empty());
    let total_supply_before = query_total_supply(&mut working_set);
    assert!(total_supply_before.is_some());

    let sender_balance_before = query_user_balance(sender_address.clone(), &mut working_set);
    let receiver_balance_before = query_user_balance(receiver_address.clone(), &mut working_set);

    assert_eq!(Some(initial_balance), sender_balance_before);
    assert!(receiver_balance_before.is_none());

    let transfer_amount = 15;
    let transfer_message = CallMessage::Transfer {
        to: receiver_address.clone(),
        coins: Coins {
            amount: transfer_amount,
            token_address: token_address.clone(),
        },
    };

    bank.call(transfer_message, &sender_context, &mut working_set)
        .expect("Transfer call failed");
    assert!(working_set.events().is_empty());

    let sender_balance_after = query_user_balance(sender_address, &mut working_set);
    let receiver_balance_after = query_user_balance(receiver_address, &mut working_set);

    assert_eq!(
        Some(initial_balance - transfer_amount),
        sender_balance_after
    );
    assert_eq!(Some(transfer_amount), receiver_balance_after);
    let total_supply_after = query_total_supply(&mut working_set);
    assert_eq!(total_supply_before, total_supply_after);
}
