use sov_modules_api::prelude::*;
use sov_modules_api::sov_universal_wallet::schema::Schema;
use sov_modules_api::Spec;
use sov_test_utils::TestSpec;

use crate::CallMessage;

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
