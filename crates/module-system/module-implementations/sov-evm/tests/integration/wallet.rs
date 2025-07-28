use borsh::BorshDeserialize;
use sov_evm::{CallMessage, RlpEvmTransaction};
use sov_modules_api::macros::UniversalWallet;
use sov_modules_api::sov_universal_wallet::schema::Schema;

#[derive(Debug, Clone, PartialEq, borsh::BorshSerialize, BorshDeserialize, UniversalWallet)]
pub enum RuntimeCall {
    Evm(CallMessage),
}

#[test]
fn test_display_evm() {
    let msg: RuntimeCall = RuntimeCall::Evm(CallMessage {
        rlp: RlpEvmTransaction { rlp: vec![1, 2, 3] },
    });
    let schema = Schema::of_single_type::<RuntimeCall>().unwrap();
    assert_eq!(
        schema.display(0, &borsh::to_vec(&msg).unwrap()).unwrap(),
        r#"Evm { rlp: { rlp: 0x010203 } }"#
    );
}
