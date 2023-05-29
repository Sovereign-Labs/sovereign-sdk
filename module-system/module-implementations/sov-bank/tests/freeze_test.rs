use helpers::{generate_address, C};
use sov_bank::call::CallMessage;
use sov_bank::genesis::{DEPLOYER, SALT};
use sov_bank::query::TotalSupplyResponse;
use sov_bank::{create_token_address, Bank, BankConfig, Coins};
use sov_modules_api::{Address, Context, Module, ModuleInfo};
use sov_state::{DefaultStorageSpec, ProverStorage, WorkingSet};

use crate::helpers::create_bank_config_with_token;

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

    let freeze = bank
        .call(freeze_message.clone(), &minter_context, &mut working_set);
    assert!(freeze.is_err());

    assert_eq!("Token is already frozen".to_string(),freeze.err().unwrap().to_string());


}