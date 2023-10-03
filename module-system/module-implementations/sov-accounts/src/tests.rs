use std::str::FromStr;

use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::default_signature::private_key::DefaultPrivateKey;
use sov_modules_api::default_signature::DefaultPublicKey;
use sov_modules_api::{AddressBech32, Context, Module, PrivateKey, PublicKey, Spec, WorkingSet};
use sov_state::ProverStorage;

use crate::query::{self, Response};
use crate::{call, AccountConfig, Accounts};
type C = DefaultContext;

#[test]
fn test_config_serialization() {
    let pub_key = &DefaultPublicKey::from_str(
        "1cd4e2d9d5943e6f3d12589d31feee6bb6c11e7b8cd996a393623e207da72cbf",
    )
    .unwrap();

    let config = AccountConfig::<DefaultContext> {
        pub_keys: vec![pub_key.clone()],
    };

    let data = r#"
    {
        "pub_keys":["1cd4e2d9d5943e6f3d12589d31feee6bb6c11e7b8cd996a393623e207da72cbf"]
    }"#;

    let parsed_config: AccountConfig<DefaultContext> = serde_json::from_str(data).unwrap();
    assert_eq!(parsed_config, config);
}

#[test]
fn test_config_account() {
    let priv_key = DefaultPrivateKey::generate();
    let init_pub_key = priv_key.pub_key();
    let init_pub_key_addr = init_pub_key.to_address::<<C as Spec>::Address>();

    let account_config = AccountConfig {
        pub_keys: vec![init_pub_key.clone()],
    };

    let accounts = &mut Accounts::<C>::default();
    let tmpdir = tempfile::tempdir().unwrap();
    let native_working_set = &mut WorkingSet::new(ProverStorage::with_path(tmpdir.path()).unwrap());

    accounts
        .init_module(&account_config, native_working_set)
        .unwrap();

    let query_response = accounts
        .get_account(init_pub_key, native_working_set)
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
    let tmpdir = tempfile::tempdir().unwrap();
    let native_working_set = &mut WorkingSet::new(ProverStorage::with_path(tmpdir.path()).unwrap());
    let accounts = &mut Accounts::<C>::default();

    let priv_key = DefaultPrivateKey::generate();

    let sender = priv_key.pub_key();
    let sender_addr = sender.to_address::<<C as Spec>::Address>();
    let sender_context = C::new(sender_addr);

    // Test new account creation
    {
        accounts
            .create_default_account(&sender, native_working_set)
            .unwrap();

        let query_response = accounts
            .get_account(sender.clone(), native_working_set)
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
        let priv_key = DefaultPrivateKey::generate();
        let new_pub_key = priv_key.pub_key();
        let sig = priv_key.sign(&call::UPDATE_ACCOUNT_MSG);
        accounts
            .call(
                call::CallMessage::<C>::UpdatePublicKey(new_pub_key.clone(), sig),
                &sender_context,
                native_working_set,
            )
            .unwrap();

        // Account corresponding to the old public key does not exist
        let query_response = accounts.get_account(sender, native_working_set).unwrap();

        assert_eq!(query_response, query::Response::AccountEmpty);

        // New account with the new public key and an old address is created.
        let query_response = accounts
            .get_account(new_pub_key, native_working_set)
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
    let tmpdir = tempfile::tempdir().unwrap();
    let native_working_set = &mut WorkingSet::new(ProverStorage::with_path(tmpdir.path()).unwrap());
    let accounts = &mut Accounts::<C>::default();

    let sender_1 = DefaultPrivateKey::generate().pub_key();
    let sender_context_1 = C::new(sender_1.to_address());

    accounts
        .create_default_account(&sender_1, native_working_set)
        .unwrap();

    let priv_key = DefaultPrivateKey::generate();
    let sender_2 = priv_key.pub_key();
    let sig_2 = priv_key.sign(&call::UPDATE_ACCOUNT_MSG);

    accounts
        .create_default_account(&sender_2, native_working_set)
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
fn test_get_account_after_pub_key_update() {
    let tmpdir = tempfile::tempdir().unwrap();
    let native_working_set = &mut WorkingSet::new(ProverStorage::with_path(tmpdir.path()).unwrap());
    let accounts = &mut Accounts::<C>::default();

    let sender_1 = DefaultPrivateKey::generate().pub_key();
    let sender_1_addr = sender_1.to_address::<<C as Spec>::Address>();
    let sender_context_1 = C::new(sender_1_addr);

    accounts
        .create_default_account(&sender_1, native_working_set)
        .unwrap();

    let priv_key = DefaultPrivateKey::generate();
    let new_pub_key = priv_key.pub_key();
    let sig = priv_key.sign(&call::UPDATE_ACCOUNT_MSG);
    accounts
        .call(
            call::CallMessage::<C>::UpdatePublicKey(new_pub_key.clone(), sig),
            &sender_context_1,
            native_working_set,
        )
        .unwrap();

    let acc = accounts
        .accounts
        .get(&new_pub_key, native_working_set)
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
        r#"{"AccountExists":{"addr":"sov1qypqxpq9qcrsszg2pvxq6rs0zqg3yyc5z5tpwxqergd3c8g7rusqqsn6hm","nonce":123456789}}"#
    );
}

#[test]
fn test_response_deserialization() {
    let json = r#"{"AccountExists":{"addr":"sov1qypqxpq9qcrsszg2pvxq6rs0zqg3yyc5z5tpwxqergd3c8g7rusqqsn6hm","nonce":123456789}}"#;
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
        Ok(response) => panic!("Expected error, got {:?}", response),
        Err(err) => {
            assert_eq!(err.to_string(), "Wrong HRP: hax at line 1 column 42");
        }
    }
}
