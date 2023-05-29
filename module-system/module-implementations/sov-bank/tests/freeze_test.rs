use helpers::{generate_address, C};
use sov_bank::call::CallMessage;
use sov_bank::query::TotalSupplyResponse;
use sov_bank::{create_token_address, Bank, BankConfig, Coins};
use sov_modules_api::{Address, Context, Module, ModuleInfo};
use sov_state::{DefaultStorageSpec, ProverStorage, WorkingSet};

mod helpers;

pub type Storage = ProverStorage<DefaultStorageSpec>;

#[test]
fn freeze_token() {
    let bank = Bank::<C>::new();
    let mut working_set = WorkingSet::new(ProverStorage::temporary());
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
    };
    let minted = bank
        .call(mint_message, &minter_context, &mut working_set)
        .expect("Failed to mint token");
    // No events at the moment. If there are, needs to be checked
    assert!(minted.events.is_empty());

    // -----
    // Freeze
    let freeze_message = CallMessage::Freeze {
        token_address: token_address.clone(),
    };

    let freeze = bank
        .call(freeze_message.clone(), &minter_context, &mut working_set)
        .expect("Failed to burn token");
    assert!(freeze.events.is_empty());

    // ----
    // Try to freeze an already frozen token
    let freeze_message = CallMessage::Freeze {
        token_address: token_address.clone(),
    };

    let freeze = bank.call(freeze_message.clone(), &minter_context, &mut working_set);
    assert!(freeze.is_err());

    assert_eq!(
        "Token is already frozen".to_string(),
        freeze.err().unwrap().to_string()
    );

    // create a second token
    let token_name = "Token2".to_owned();
    let initial_balance = 100;
    let token_address_2 = create_token_address::<C>(&token_name, minter_address.as_ref(), salt);

    // ---
    // Deploying second token
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

    // Try to freeze with a non authorized minter
    let unauthorized_address = generate_address("unauthorized_address");
    let unauthorized_context = C::new(unauthorized_address.clone());
    let freeze_message = CallMessage::Freeze {
        token_address: token_address_2.clone(),
    };

    let freeze = bank.call(
        freeze_message.clone(),
        &unauthorized_context,
        &mut working_set,
    );
    assert!(freeze.is_err());
    let unauthorized_minter_msg = format!(
        "Sender {} is not an authorized minter",
        unauthorized_address
    );
    assert_eq!(unauthorized_minter_msg, freeze.err().unwrap().to_string());

    // -----
    // Try to mint a frozen token
    let mint_amount = 10;
    let new_holder = generate_address("new_holder");
    let mint_message = CallMessage::Mint {
        coins: Coins {
            amount: mint_amount,
            token_address: token_address.clone(),
        },
        minter_address: new_holder.clone(),
    };

    let query_total_supply = |token_address: Address,
                              working_set: &mut WorkingSet<Storage>|
     -> Option<u64> {
        let total_supply: TotalSupplyResponse = bank.supply_of(token_address.clone(), working_set);
        total_supply.amount
    };

    let minted = bank.call(mint_message.clone(), &minter_context, &mut working_set);
    assert!(minted.is_err());

    assert_eq!(
        "Attempt to mint frozen token".to_string(),
        minted.err().unwrap().to_string()
    );

    // -----
    // Try to mint an unfrozen token, sanity check
    let mint_amount = 10;
    let mint_message = CallMessage::Mint {
        coins: Coins {
            amount: mint_amount,
            token_address: token_address_2.clone(),
        },
        minter_address: minter_address.clone(),
    };

    let minted = bank
        .call(mint_message.clone(), &minter_context, &mut working_set)
        .expect("Failed to mint token");
    assert!(minted.events.is_empty());

    let total_supply = query_total_supply(token_address_2.clone(), &mut working_set);
    assert_eq!(Some(initial_balance + mint_amount), total_supply);

    let query_user_balance = |token_address: Address,
                              user_address: Address,
                              working_set: &mut WorkingSet<Storage>|
     -> Option<u64> {
        bank.get_balance_of(user_address, token_address.clone(), working_set)
    };
    let bal = query_user_balance(token_address_2.clone(), minter_address, &mut working_set);

    assert_eq!(Some(110), bal);
}
