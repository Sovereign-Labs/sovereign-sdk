use helpers::{generate_address, C};
use sov_bank::call::CallMessage;
use sov_bank::query::TotalSupplyResponse;
use sov_bank::{create_token_address, Bank, BankConfig, Coins};
use sov_modules_api::{Address, Context, Module, ModuleInfo};
use sov_state::{DefaultStorageSpec, ProverStorage, WorkingSet};

mod helpers;

pub type Storage = ProverStorage<DefaultStorageSpec>;

#[test]
fn mint_token() {
    let bank = Bank::<C>::new();
    let tmpdir = tempfile::tempdir().unwrap();
    let mut working_set = WorkingSet::new(ProverStorage::with_path(tmpdir.path()).unwrap());
    let empty_bank_config = BankConfig::<C> { tokens: vec![] };
    bank.genesis(&empty_bank_config, &mut working_set).unwrap();

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
        authorized_minters: vec![minter_address.clone()],
    };
    let _minted = bank
        .call(mint_message, &minter_context, &mut working_set)
        .expect("Failed to mint token");
    // No events at the moment. If there are, needs to be checked
    assert!(working_set.events().is_empty());

    let query_total_supply = |token_address: Address,
                              working_set: &mut WorkingSet<Storage>|
     -> Option<u64> {
        let total_supply: TotalSupplyResponse = bank.supply_of(token_address.clone(), working_set);
        total_supply.amount
    };

    let query_user_balance =
        |user_address: Address, working_set: &mut WorkingSet<Storage>| -> Option<u64> {
            bank.get_balance_of(user_address, token_address.clone(), working_set)
        };

    let previous_total_supply = query_total_supply(token_address.clone(), &mut working_set);
    assert_eq!(Some(initial_balance), previous_total_supply);

    // -----
    // Mint Additional
    let mint_amount = 10;
    let new_holder = generate_address("new_holder");
    let mint_message = CallMessage::Mint {
        coins: Coins {
            amount: mint_amount,
            token_address: token_address.clone(),
        },
        minter_address: new_holder.clone(),
    };

    let _minted = bank
        .call(mint_message.clone(), &minter_context, &mut working_set)
        .expect("Failed to mint token");
    assert!(working_set.events().is_empty());

    let total_supply = query_total_supply(token_address.clone(), &mut working_set);
    assert_eq!(Some(initial_balance + mint_amount), total_supply);

    // check user balance after minting
    let bal = query_user_balance(new_holder.clone(), &mut working_set);
    assert_eq!(Some(10), bal);

    // check original token creation balance
    let bal = query_user_balance(minter_address.clone(), &mut working_set);
    assert_eq!(Some(100), bal);

    // Mint with an un-authorized user
    let unauthorized_address = generate_address("unauthorized_address");
    let unauthorized_context = C::new(unauthorized_address.clone());
    let unauthorized_mint = bank.call(
        mint_message.clone(),
        &unauthorized_context,
        &mut working_set,
    );

    assert!(unauthorized_mint.is_err());
    let expected_error = format!(
        "Sender {} is not an authorized minter",
        unauthorized_address
    );
    let actual_msg = unauthorized_mint.err().unwrap().to_string();
    assert!(actual_msg.contains(&expected_error));

    // Authorized minter test
    let salt = 0;
    let token_name = "Token_New".to_owned();
    let initial_balance = 100;
    let token_address = create_token_address::<C>(&token_name, minter_address.as_ref(), salt);
    let authorized_minter_address_1 = generate_address("authorized_minter_1");
    let authorized_minter_address_2 = generate_address("authorized_minter_2");
    // ---
    // Deploying token
    let mint_message = CallMessage::CreateToken {
        salt,
        token_name,
        initial_balance,
        minter_address: minter_address.clone(),
        authorized_minters: vec![
            authorized_minter_address_1.clone(),
            authorized_minter_address_2.clone(),
        ],
    };
    let _minted = bank
        .call(mint_message, &minter_context, &mut working_set)
        .expect("Failed to mint token");
    // No events at the moment. If there are, needs to be checked
    assert!(working_set.events().is_empty());

    // Try to mint new token with original token creator, in this case minter_context
    let mint_amount = 10;
    let new_holder = generate_address("new_holder_2");
    let mint_message = CallMessage::Mint {
        coins: Coins {
            amount: mint_amount,
            token_address: token_address.clone(),
        },
        minter_address: new_holder.clone(),
    };

    let minted = bank.call(mint_message.clone(), &minter_context, &mut working_set);
    let err = format!(
        "Sender {} is not an authorized minter",
        minter_address.clone()
    );
    assert!(minted.is_err());
    assert_eq!(err, minted.err().unwrap().to_string());

    // Try to mint new token with authorized sender 2
    let authorized_minter_2_context = C::new(authorized_minter_address_2.clone());
    let mint_message = CallMessage::Mint {
        coins: Coins {
            amount: mint_amount,
            token_address: token_address.clone(),
        },
        minter_address: new_holder.clone(),
    };

    let _minted = bank
        .call(
            mint_message.clone(),
            &authorized_minter_2_context,
            &mut working_set,
        )
        .expect("Failed to mint token");
    let supply = query_total_supply(token_address.clone(), &mut working_set);
    assert!(working_set.events().is_empty());
    assert_eq!(Some(110), supply);

    // Try to mint new token with authorized sender 1
    let authorized_minter_1_context = C::new(authorized_minter_address_1.clone());
    let mint_message = CallMessage::Mint {
        coins: Coins {
            amount: mint_amount,
            token_address: token_address.clone(),
        },
        minter_address: new_holder.clone(),
    };

    let _minted = bank
        .call(
            mint_message.clone(),
            &authorized_minter_1_context,
            &mut working_set,
        )
        .expect("Failed to mint token");
    let supply = query_total_supply(token_address.clone(), &mut working_set);
    assert!(working_set.events().is_empty());
    assert_eq!(Some(120), supply);

    // Overflow test - account balance
    let overflow_mint_message = CallMessage::Mint {
        coins: Coins {
            amount: u64::MAX,
            token_address: token_address.clone(),
        },
        minter_address: new_holder.clone(),
    };

    let minted = bank.call(
        overflow_mint_message.clone(),
        &authorized_minter_1_context,
        &mut working_set,
    );
    assert!(minted.is_err());
    assert_eq!(
        "Account Balance overflow in the mint method of bank module",
        minted.err().unwrap().to_string()
    );
    // assert that the supply is unchanged after the overflow mint
    let supply = query_total_supply(token_address.clone(), &mut working_set);
    assert_eq!(Some(120), supply);

    // Overflow test 2 - total supply
    let new_holder = generate_address("new_holder_3");
    let overflow_mint_message = CallMessage::Mint {
        coins: Coins {
            amount: u64::MAX - 1,
            token_address: token_address.clone(),
        },
        minter_address: new_holder.clone(),
    };

    let minted = bank.call(
        overflow_mint_message.clone(),
        &authorized_minter_1_context,
        &mut working_set,
    );
    assert!(minted.is_err());
    assert_eq!(
        "Total Supply overflow in the mint method of bank module",
        minted.err().unwrap().to_string()
    );

    // assert that the supply is unchanged after the overflow mint
    let supply = query_total_supply(token_address.clone(), &mut working_set);
    assert_eq!(Some(120), supply);
}
