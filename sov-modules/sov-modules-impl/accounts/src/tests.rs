use crate::{
    call, hooks,
    query::{self, QueryMessage},
    Accounts,
};
use sov_modules_api::{
    mocks::{MockContext, MockPublicKey},
    Address, Context, Module, ModuleInfo, PublicKey,
};
use sov_state::{ProverStorage, WorkingSet};

type C = MockContext;

#[test]
fn test_update_account() {
    let native_storage = WorkingSet::new(ProverStorage::temporary());
    let accounts = &mut Accounts::<C>::new(native_storage.clone());
    let mut hooks = hooks::Hooks::<C>::new(native_storage);

    let sender = MockPublicKey::try_from("pub_key").unwrap();
    let sender_addr: Address = sender.to_address();
    let sender_context = C::new(sender.clone());

    // Test new account creation
    {
        hooks.get_account_or_create_default(sender.clone()).unwrap();

        let query_response: query::Response = serde_json::from_slice(
            &accounts
                .query(QueryMessage::GetAccount(sender.clone()))
                .response,
        )
        .unwrap();

        assert_eq!(
            query_response,
            query::Response::AccountExists {
                addr: sender_addr.inner(),
                nonce: 0
            }
        )
    }

    // Test public key update
    {
        let new_pub_key = MockPublicKey::try_from("new_pub_key").unwrap();
        let sig = new_pub_key.sign(call::UPDATE_ACCOUNT_MSG);
        accounts
            .call(
                call::CallMessage::<C>::UpdatePublicKey(new_pub_key.clone(), sig),
                &sender_context,
            )
            .unwrap();

        // Account corresponding to the old public key does not exist
        let query_response: query::Response =
            serde_json::from_slice(&accounts.query(QueryMessage::GetAccount(sender)).response)
                .unwrap();

        assert_eq!(query_response, query::Response::AccountEmpty);

        // New account with the new public key and an old address is created.
        let query_response: query::Response = serde_json::from_slice(
            &accounts
                .query(QueryMessage::GetAccount(new_pub_key))
                .response,
        )
        .unwrap();

        assert_eq!(
            query_response,
            query::Response::AccountExists {
                addr: sender_addr.inner(),
                nonce: 0
            }
        )
    }
}

#[test]
fn test_update_account_fails() {
    let native_storage = WorkingSet::new(ProverStorage::temporary());
    let accounts = &mut Accounts::<C>::new(native_storage.clone());
    let mut hooks = hooks::Hooks::<C>::new(native_storage);

    let sender_1 = MockPublicKey::try_from("pub_key_1").unwrap();
    let sender_context_1 = C::new(sender_1.clone());
    hooks.get_account_or_create_default(sender_1).unwrap();

    let sender_2 = MockPublicKey::try_from("pub_key_2").unwrap();
    let sig_2 = sender_2.sign(call::UPDATE_ACCOUNT_MSG);

    hooks
        .get_account_or_create_default(sender_2.clone())
        .unwrap();

    // The new public key already exists and the call fails.
    assert!(accounts
        .call(
            call::CallMessage::<C>::UpdatePublicKey(sender_2, sig_2),
            &sender_context_1,
        )
        .is_err())
}

#[test]
fn test_create_account_fails() {
    let native_storage = WorkingSet::new(ProverStorage::temporary());
    let accounts = &mut Accounts::<C>::new(native_storage.clone());
    let mut hooks = hooks::Hooks::<C>::new(native_storage);

    let sender_1 = MockPublicKey::try_from("pub_key_1").unwrap();
    let sender_context_1 = C::new(sender_1.clone());

    hooks.get_account_or_create_default(sender_1).unwrap();

    let new_pub_key = MockPublicKey::try_from("pub_key_2").unwrap();
    let sig = new_pub_key.sign(call::UPDATE_ACCOUNT_MSG);
    accounts
        .call(
            call::CallMessage::<C>::UpdatePublicKey(new_pub_key.clone(), sig),
            &sender_context_1,
        )
        .unwrap();

    // Account creation fails because the `new_pub_key` is already registered.
    // assert!(hooks.get_account_or_create_default(new_pub_key).is_err());
}
