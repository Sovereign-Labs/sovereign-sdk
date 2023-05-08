use crate::{
    call, hooks,
    query::{self, QueryMessage, Response},
    AccountConfig, Accounts,
};
use sov_modules_api::{
    default_context::DefaultContext, mocks::DefaultPublicKey, AddressBech32, Context, Module,
    ModuleInfo, PublicKey, Spec,
};
use sov_state::{ProverStorage, WorkingSet};

type C = DefaultContext;

#[test]
fn test_config_account() {
    let init_pub_key = DefaultPublicKey::from("init_pub_key");
    let init_pub_key_addr = init_pub_key.to_address::<<C as Spec>::Address>();

    let account_config = AccountConfig::<C> {
        pub_keys: vec![init_pub_key.clone()],
    };

    let accounts = &mut Accounts::<C>::new();
    let native_working_set = &mut WorkingSet::new(ProverStorage::temporary());

    accounts
        .init_module(&account_config, native_working_set)
        .unwrap();

    let query_response: query::Response = serde_json::from_slice(
        &accounts
            .query(
                QueryMessage::GetAccount(init_pub_key.clone()),
                native_working_set,
            )
            .response,
    )
    .unwrap();

    assert_eq!(
        query_response,
        query::Response::AccountExists {
            addr: AddressBech32::from(&init_pub_key_addr),
            nonce: 0
        }
    )
}

#[test]
fn test_update_account() {
    let native_working_set = &mut WorkingSet::new(ProverStorage::temporary());
    let accounts = &mut Accounts::<C>::new();
    let hooks = hooks::Hooks::<C>::new();

    let sender = DefaultPublicKey::from("pub_key");
    let sender_addr = sender.to_address::<<C as Spec>::Address>();
    let sender_context = C::new(sender_addr.clone());

    // Test new account creation
    {
        hooks
            .get_or_create_default_account(sender.clone(), native_working_set)
            .unwrap();

        let query_response: query::Response = serde_json::from_slice(
            &accounts
                .query(QueryMessage::GetAccount(sender.clone()), native_working_set)
                .response,
        )
        .unwrap();

        assert_eq!(
            query_response,
            query::Response::AccountExists {
                addr: AddressBech32::try_from(sender_addr.as_ref()).unwrap(),
                nonce: 0
            }
        )
    }

    // Test public key update
    {
        let new_pub_key = DefaultPublicKey::from("new_pub_key");
        let sig = new_pub_key.sign(call::UPDATE_ACCOUNT_MSG);
        accounts
            .call(
                call::CallMessage::<C>::UpdatePublicKey(new_pub_key.clone(), sig),
                &sender_context,
                native_working_set,
            )
            .unwrap();

        // Account corresponding to the old public key does not exist
        let query_response: query::Response = serde_json::from_slice(
            &accounts
                .query(QueryMessage::GetAccount(sender), native_working_set)
                .response,
        )
        .unwrap();

        assert_eq!(query_response, query::Response::AccountEmpty);

        // New account with the new public key and an old address is created.
        let query_response: query::Response = serde_json::from_slice(
            &accounts
                .query(QueryMessage::GetAccount(new_pub_key), native_working_set)
                .response,
        )
        .unwrap();

        assert_eq!(
            query_response,
            query::Response::AccountExists {
                addr: AddressBech32::try_from(sender_addr.as_ref()).unwrap(),
                nonce: 0
            }
        )
    }
}

#[test]
fn test_update_account_fails() {
    let native_working_set = &mut WorkingSet::new(ProverStorage::temporary());
    let accounts = &mut Accounts::<C>::new();
    let hooks = hooks::Hooks::<C>::new();

    let sender_1 = DefaultPublicKey::from("pub_key_1");
    let sender_context_1 = C::new(sender_1.to_address());
    hooks
        .get_or_create_default_account(sender_1, native_working_set)
        .unwrap();

    let sender_2 = DefaultPublicKey::from("pub_key_2");
    let sig_2 = sender_2.sign(call::UPDATE_ACCOUNT_MSG);

    hooks
        .get_or_create_default_account(sender_2.clone(), native_working_set)
        .unwrap();

    // The new public key already exists and the call fails.
    assert!(accounts
        .call(
            call::CallMessage::<C>::UpdatePublicKey(sender_2, sig_2),
            &sender_context_1,
            native_working_set
        )
        .is_err())
}

#[test]
fn test_get_acc_after_pub_key_update() {
    let native_working_set = &mut WorkingSet::new(ProverStorage::temporary());
    let accounts = &mut Accounts::<C>::new();
    let hooks = hooks::Hooks::<C>::new();

    let sender_1 = DefaultPublicKey::from("pub_key_1");
    let sender_1_addr = sender_1.to_address::<<C as Spec>::Address>();
    let sender_context_1 = C::new(sender_1_addr.clone());

    hooks
        .get_or_create_default_account(sender_1, native_working_set)
        .unwrap();

    let new_pub_key = DefaultPublicKey::from("pub_key_2");
    let sig = new_pub_key.sign(call::UPDATE_ACCOUNT_MSG);
    accounts
        .call(
            call::CallMessage::<C>::UpdatePublicKey(new_pub_key.clone(), sig),
            &sender_context_1,
            native_working_set,
        )
        .unwrap();

    let acc = hooks
        .get_or_create_default_account(new_pub_key, native_working_set)
        .unwrap();
    assert_eq!(acc.addr, sender_1_addr)
}

#[test]
fn test_response_serialization() {
    let addr: Vec<u8> = (1..=32).collect();
    let nonce = 123456789;
    let response = Response::AccountExists {
        addr: AddressBech32::try_from(addr.as_slice()).unwrap(),
        nonce,
    };

    let json = serde_json::to_string(&response).unwrap();
    assert_eq!(
        json,
        r#"{"AccountExists":{"addr":"sov1qypqxpq9qcrsszg2pvxq6rs0zqg3yyc5z5tpwxqergd3c8g7rusq4vrkje","nonce":123456789}}"#
    );
}

#[test]
fn test_response_deserialization() {
    let json = r#"{"AccountExists":{"addr":"sov1qypqxpq9qcrsszg2pvxq6rs0zqg3yyc5z5tpwxqergd3c8g7rusq4vrkje","nonce":123456789}}"#;
    let response: Response = serde_json::from_str(json).unwrap();

    let expected_addr: Vec<u8> = (1..=32).collect();
    let expected_response = Response::AccountExists {
        addr: AddressBech32::try_from(expected_addr.as_slice()).unwrap(),
        nonce: 123456789,
    };

    assert_eq!(response, expected_response);
}

#[test]
fn test_response_deserialization_on_wrong_hrp() {
    let json = r#"{"AccountExists":{"addr":"hax1qypqx68ju0l","nonce":123456789}}"#;
    let response: Result<Response, serde_json::Error> = serde_json::from_str(json);
    match response {
        Ok(response) => assert!(false, "{}", format!("Expected error, got {:?}", response)),
        Err(err) => {
            assert_eq!(err.to_string(), "Wrong HRP: hax at line 1 column 42");
        }
    }
}
