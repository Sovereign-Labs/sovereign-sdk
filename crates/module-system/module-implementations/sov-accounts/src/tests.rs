use sov_modules_api::prelude::*;
use sov_modules_api::sov_universal_wallet::schema::Schema;

use crate::query::Response;
use crate::CallMessage;

type S = sov_test_utils::TestSpec;

#[test]
fn test_response_serialization() {
    let addr: Vec<u8> = (1..=28).collect();
    let mut addr_array = [0u8; 28];
    addr_array.copy_from_slice(&addr);
    let response = Response::AccountExists::<<S as Spec>::Address> {
        addr: <S as Spec>::Address::from(addr_array),
    };

    let json = serde_json::to_string(&response).unwrap();
    assert_eq!(
        json,
        r#"{"account_exists":{"addr":"sov1qypqxpq9qcrsszg2pvxq6rs0zqg3yyc5z5tpwxqergd3crhxalf"}}"#
    );
}

#[test]
fn test_response_deserialization() {
    let json =
        r#"{"account_exists":{"addr":"sov1qypqxpq9qcrsszg2pvxq6rs0zqg3yyc5z5tpwxqergd3crhxalf"}}"#;
    let response: Response<<S as Spec>::Address> = serde_json::from_str(json).unwrap();

    let expected_addr: Vec<u8> = (1..=28).collect();
    let mut addr_array = [0u8; 28];
    addr_array.copy_from_slice(&expected_addr);
    let expected_response = Response::AccountExists::<<S as Spec>::Address> {
        addr: <S as Spec>::Address::from(addr_array),
    };

    assert_eq!(response, expected_response);
}

#[test]
fn test_response_deserialization_on_wrong_hrp() {
    let json = r#"{"account_exists":{"addr":"hax1qypqx68ju0l"}}"#;
    let response: Result<Response<<S as Spec>::Address>, serde_json::Error> =
        serde_json::from_str(json);
    match response {
        Ok(response) => panic!("Expected error, got {response:?}"),
        Err(err) => {
            assert_eq!(err.to_string(), "Wrong HRP: hax at line 1 column 44");
        }
    }
}

#[test]
fn test_display_accounts_call() {
    #[derive(Debug, Clone, PartialEq, borsh::BorshSerialize, UniversalWallet)]
    enum RuntimeCall {
        Accounts(CallMessage),
    }

    let msg = RuntimeCall::Accounts(CallMessage::InsertCredentialId([1; 32].into()));

    let schema = Schema::of_single_type::<RuntimeCall>().unwrap();
    assert_eq!(
        r#"Accounts.InsertCredentialId(0x0101010101010101010101010101010101010101010101010101010101010101)"#,
        schema.display(0, &borsh::to_vec(&msg).unwrap()).unwrap(),
    );
}
