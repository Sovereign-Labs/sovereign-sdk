use sov_modules_api::prelude::*;
use sov_modules_api::sov_universal_wallet::schema::Schema;
use sov_modules_api::Spec;
use sov_test_utils::TestSpec;

use crate::{Accounts, CallMessage};

#[test]
fn test_display_accounts_call() {
    type S = TestSpec;

    #[derive(Debug, Clone, PartialEq, borsh::BorshSerialize, UniversalWallet)]
    enum RuntimeCall {
        Accounts(CallMessage<S>),
    }

    let insert_msg = RuntimeCall::Accounts(CallMessage::InsertCredentialId([1; 32].into()));
    let schema = Schema::of_single_type::<RuntimeCall>().unwrap();
    assert_eq!(
        r#"Accounts.InsertCredentialId(0x0101010101010101010101010101010101010101010101010101010101010101)"#,
        schema
            .display(0, &borsh::to_vec(&insert_msg).unwrap())
            .unwrap(),
    );

    let addr_bytes: [u8; 28] = core::array::from_fn(|i| (i + 1) as u8);
    let address = <S as Spec>::Address::from(addr_bytes);

    let add_msg = RuntimeCall::Accounts(CallMessage::AddCredentialToAddress {
        address,
        credential: [2; 32].into(),
    });
    let rendered = schema
        .display(0, &borsh::to_vec(&add_msg).unwrap())
        .unwrap();
    assert!(
        rendered.starts_with("Accounts.AddCredentialToAddress"),
        "unexpected render: {rendered}"
    );
    assert!(
        rendered.contains("0x0202020202020202020202020202020202020202020202020202020202020202"),
        "render missing credential: {rendered}"
    );

    let remove_msg = RuntimeCall::Accounts(CallMessage::RemoveCredentialFromAddress {
        address,
        credential: [3; 32].into(),
    });
    let rendered = schema
        .display(0, &borsh::to_vec(&remove_msg).unwrap())
        .unwrap();
    assert!(
        rendered.starts_with("Accounts.RemoveCredentialFromAddress"),
        "unexpected render: {rendered}"
    );
    assert!(
        rendered.contains("0x0303030303030303030303030303030303030303030303030303030303030303"),
        "render missing credential: {rendered}"
    );

    let rotate_msg = RuntimeCall::Accounts(CallMessage::RotateCredentialOnAddress {
        address,
        old_credential: [4; 32].into(),
        new_credential: [5; 32].into(),
    });
    let rendered = schema
        .display(0, &borsh::to_vec(&rotate_msg).unwrap())
        .unwrap();
    assert!(
        rendered.starts_with("Accounts.RotateCredentialOnAddress"),
        "unexpected render: {rendered}"
    );
    assert!(
        rendered.contains("0x0404040404040404040404040404040404040404040404040404040404040404"),
        "render missing old_credential: {rendered}"
    );
    assert!(
        rendered.contains("0x0505050505050505050505050505050505050505050505050505050505050505"),
        "render missing new_credential: {rendered}"
    );
}

/// `caller_credential_id` must reject any sender credential that is not a
/// `CryptoSpec::PublicKey` or `Multisig`. We exercise the rejection path by
/// handing `Context::new` an empty `Credentials` bag, which is the same shape
/// the function sees if a future authenticator stores some unknown third
/// credential type.
#[test]
fn test_create_synthetic_address_rejects_unsupported_credential() {
    use sov_modules_api::transaction::Credentials;
    use sov_modules_api::{Context, ExecutionContext, SequencerType};

    type S = TestSpec;

    let sender_bytes: [u8; 28] = core::array::from_fn(|i| (i + 1) as u8);
    let sender = <S as Spec>::Address::from(sender_bytes);
    let da_address = <<S as Spec>::Da as sov_modules_api::DaSpec>::Address::default();

    let ctx = Context::<S>::new(
        sender,
        Credentials::default(),
        sender,
        da_address,
        None,
        ExecutionContext::Node,
        SequencerType::Preferred,
    );

    let accounts = Accounts::<S>::default();
    let err = accounts
        .caller_credential_id(&ctx)
        .expect_err("empty credentials should not resolve to a credential id");
    let msg = err.to_string();
    assert!(
        msg.contains("CreateSyntheticAddress"),
        "error should attribute to the CreateSyntheticAddress call message, got: {msg}"
    );
}
