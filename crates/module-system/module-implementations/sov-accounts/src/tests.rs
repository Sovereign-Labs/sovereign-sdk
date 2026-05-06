use sov_modules_api::prelude::*;
use sov_modules_api::sov_universal_wallet::schema::Schema;

use crate::CallMessage;

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
