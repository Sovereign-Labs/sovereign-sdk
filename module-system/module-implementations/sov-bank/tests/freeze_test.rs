use helpers::C;
use sov_bank::{get_token_address, Bank, BankConfig, CallMessage, Coins, TotalSupplyResponse};
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::utils::generate_address;
use sov_modules_api::{Address, Context, Error, Module, WorkingSet};
use sov_state::{DefaultStorageSpec, ProverStorage};

mod helpers;

pub type Storage = ProverStorage<DefaultStorageSpec>;

#[test]
fn freeze_token() {
    let bank = Bank::<C>::default();
    let tmpdir = tempfile::tempdir().unwrap();
    let mut working_set = WorkingSet::new(ProverStorage::with_path(tmpdir.path()).unwrap());
    let empty_bank_config = BankConfig::<C> { tokens: vec![] };
    bank.genesis(&empty_bank_config, &mut working_set).unwrap();

    let minter_address = generate_address::<DefaultContext>("minter");
    let minter_context = C::new(minter_address);

    let salt = 0;
    let token_name = "Token1".to_owned();
    let initial_balance = 100;
    let token_address = get_token_address::<C>(&token_name, minter_address.as_ref(), salt);

    // ---
    // Deploying token
    let mint_message = CallMessage::CreateToken {
        salt,
        token_name: token_name.clone(),
        initial_balance,
        minter_address,
        authorized_minters: vec![minter_address],
    };
    let _minted = bank
        .call(mint_message, &minter_context, &mut working_set)
        .expect("Failed to mint token");
    // No events at the moment. If there are, needs to be checked
    assert!(working_set.events().is_empty());

    // -----
    // Freeze
    let freeze_message = CallMessage::Freeze { token_address };

    let _freeze = bank
        .call(freeze_message, &minter_context, &mut working_set)
        .expect("Failed to freeze token");
    assert!(working_set.events().is_empty());

    // ----
    // Try to freeze an already frozen token
    let freeze_message = CallMessage::Freeze { token_address };

    let freeze = bank.call(freeze_message, &minter_context, &mut working_set);
    assert!(freeze.is_err());
    let Error::ModuleError(err) = freeze.err().unwrap();
    let mut chain = err.chain();
    let message_1 = chain.next().unwrap().to_string();
    let message_2 = chain.next().unwrap().to_string();
    assert!(chain.next().is_none());
    assert_eq!(
        format!(
            "Failed freeze token_address={} by sender {}",
            token_address, minter_address
        ),
        message_1
    );
    assert_eq!(format!("Token {} is already frozen", token_name), message_2);

    // create a second token
    let token_name_2 = "Token2".to_owned();
    let initial_balance = 100;
    let token_address_2 = get_token_address::<C>(&token_name_2, minter_address.as_ref(), salt);

    // ---
    // Deploying second token
    let mint_message = CallMessage::CreateToken {
        salt,
        token_name: token_name_2.clone(),
        initial_balance,
        minter_address,
        authorized_minters: vec![minter_address],
    };
    let _minted = bank
        .call(mint_message, &minter_context, &mut working_set)
        .expect("Failed to mint token");
    // No events at the moment. If there are, needs to be checked
    assert!(working_set.events().is_empty());

    // Try to freeze with a non authorized minter
    let unauthorized_address = generate_address::<C>("unauthorized_address");
    let unauthorized_context = C::new(unauthorized_address);
    let freeze_message = CallMessage::Freeze {
        token_address: token_address_2,
    };

    let freeze = bank.call(freeze_message, &unauthorized_context, &mut working_set);
    assert!(freeze.is_err());
    let Error::ModuleError(err) = freeze.err().unwrap();
    let mut chain = err.chain();
    let message_1 = chain.next().unwrap().to_string();
    let message_2 = chain.next().unwrap().to_string();
    assert!(chain.next().is_none());
    assert_eq!(
        format!(
            "Failed freeze token_address={} by sender {}",
            token_address_2, unauthorized_address
        ),
        message_1
    );
    assert_eq!(
        format!(
            "Sender {} is not an authorized minter of token {}",
            unauthorized_address, token_name_2
        ),
        message_2
    );

    // Try to mint a frozen token
    let mint_amount = 10;
    let new_holder = generate_address::<C>("new_holder");
    let mint_message = CallMessage::Mint {
        coins: Coins {
            amount: mint_amount,
            token_address,
        },
        minter_address: new_holder,
    };

    let query_total_supply = |token_address: Address,
                              working_set: &mut WorkingSet<DefaultContext>|
     -> Option<u64> {
        let total_supply: TotalSupplyResponse = bank.supply_of(token_address, working_set).unwrap();
        total_supply.amount
    };

    let minted = bank.call(mint_message, &minter_context, &mut working_set);
    assert!(minted.is_err());

    let Error::ModuleError(err) = minted.err().unwrap();
    let mut chain = err.chain();
    let message_1 = chain.next().unwrap().to_string();
    let message_2 = chain.next().unwrap().to_string();
    assert!(chain.next().is_none());
    assert_eq!(
        format!(
            "Failed mint coins(token_address={} amount={}) to {} by authorizer {}",
            token_address, mint_amount, new_holder, minter_address
        ),
        message_1
    );
    assert_eq!(
        format!("Attempt to mint frozen token {}", token_name),
        message_2
    );

    // -----
    // Try to mint an unfrozen token, sanity check
    let mint_amount = 10;
    let mint_message = CallMessage::Mint {
        coins: Coins {
            amount: mint_amount,
            token_address: token_address_2,
        },
        minter_address,
    };

    let _minted = bank
        .call(mint_message, &minter_context, &mut working_set)
        .expect("Failed to mint token");
    assert!(working_set.events().is_empty());

    let total_supply = query_total_supply(token_address_2, &mut working_set);
    assert_eq!(Some(initial_balance + mint_amount), total_supply);

    let query_user_balance =
        |token_address: Address,
         user_address: Address,
         working_set: &mut WorkingSet<DefaultContext>|
         -> Option<u64> { bank.get_balance_of(user_address, token_address, working_set) };
    let bal = query_user_balance(token_address_2, minter_address, &mut working_set);

    assert_eq!(Some(110), bal);
}
