use sov_bank::{get_token_address, Bank, CallMessage};
use sov_modules_api::utils::generate_address;
use sov_modules_api::{Context, Module, WorkingSet};
use sov_state::ProverStorage;

mod helpers;

use helpers::*;

#[test]
fn initial_and_deployed_token() {
    let bank_config = create_bank_config_with_token(1, 100);
    let tmpdir = tempfile::tempdir().unwrap();
    let mut working_set = WorkingSet::new(ProverStorage::with_path(tmpdir.path()).unwrap());
    let bank = Bank::default();
    bank.genesis(&bank_config, &mut working_set).unwrap();

    let sender_address = generate_address::<C>("sender");
    let sender_context = C::new(sender_address);
    let minter_address = generate_address::<C>("minter");
    let initial_balance = 500;
    let token_name = "Token1".to_owned();
    let salt = 1;
    let token_address = get_token_address::<C>(&token_name, sender_address.as_ref(), salt);
    let create_token_message = CallMessage::CreateToken::<C> {
        salt,
        token_name: token_name.clone(),
        initial_balance,
        minter_address,
        authorized_minters: vec![minter_address],
    };

    bank.call(create_token_message, &sender_context, &mut working_set)
        .expect("Failed to create token");

    assert!(working_set.events().is_empty());

    let sender_balance = bank.get_balance_of(sender_address, token_address, &mut working_set);
    assert!(sender_balance.is_none());

    let observed_token_name = bank
        .get_token_name(&token_address, &mut working_set)
        .expect("Token is missing its name");
    assert_eq!(&token_name, &observed_token_name);

    let minter_balance = bank.get_balance_of(minter_address, token_address, &mut working_set);

    assert_eq!(Some(initial_balance), minter_balance);
}

#[test]
/// Currently integer overflow happens on bank genesis
fn overflow_max_supply() {
    let bank = Bank::<C>::default();
    let tmpdir = tempfile::tempdir().unwrap();
    let mut working_set = WorkingSet::new(ProverStorage::with_path(tmpdir.path()).unwrap());

    let bank_config = create_bank_config_with_token(2, u64::MAX - 2);

    let genesis_result = bank.genesis(&bank_config, &mut working_set);
    assert!(genesis_result.is_err());

    assert_eq!(
        "Total supply overflow",
        genesis_result.unwrap_err().to_string()
    );
}
