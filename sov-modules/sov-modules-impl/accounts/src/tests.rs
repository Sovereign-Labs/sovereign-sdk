use crate::{
    call, hooks,
    query::{self, QueryMessage},
    Accounts,
};
use sov_modules_api::{
    mocks::{MockContext, MockPublicKey},
    Address, Context, Module, ModuleInfo, PublicKey, Spec,
};
use sov_state::{ProverStorage, WorkingSet};

type C = MockContext;

#[test]
fn test_update_account() {
    let native_working_set = &mut WorkingSet::new(ProverStorage::temporary());
    let accounts = &mut Accounts::<C>::new();
    let mut hooks = hooks::Hooks::<C>::new();

    let sender = MockPublicKey::try_from("pub_key").unwrap();
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
                addr: sender_addr.as_bytes().to_vec(),
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
                addr: sender_addr.as_bytes().to_vec(),
                nonce: 0
            }
        )
    }
}

#[test]
fn test_update_account_fails() {
    let native_working_set = &mut WorkingSet::new(ProverStorage::temporary());
    let accounts = &mut Accounts::<C>::new();
    let mut hooks = hooks::Hooks::<C>::new();

    let sender_1 = MockPublicKey::try_from("pub_key_1").unwrap();
    let sender_context_1 = C::new(sender_1.to_address());
    hooks
        .get_or_create_default_account(sender_1, native_working_set)
        .unwrap();

    let sender_2 = MockPublicKey::try_from("pub_key_2").unwrap();
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
    let mut hooks = hooks::Hooks::<C>::new();

    let sender_1 = MockPublicKey::try_from("pub_key_1").unwrap();
    let sender_1_addr = sender_1.to_address::<<C as Spec>::Address>();
    let sender_context_1 = C::new(sender_1_addr.clone());

    hooks
        .get_or_create_default_account(sender_1, native_working_set)
        .unwrap();

    let new_pub_key = MockPublicKey::try_from("pub_key_2").unwrap();
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
