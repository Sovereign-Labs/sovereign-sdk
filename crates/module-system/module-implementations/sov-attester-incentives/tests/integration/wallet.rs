use borsh::BorshDeserialize;
use sov_attester_incentives::CallMessage;
use sov_modules_api::macros::UniversalWallet;
use sov_modules_api::sov_universal_wallet::schema::Schema;
use sov_modules_api::Amount;

#[derive(Debug, Clone, PartialEq, borsh::BorshSerialize, BorshDeserialize, UniversalWallet)]
pub enum RuntimeCall {
    AttesterIncentives(CallMessage),
}

#[test]
fn test_display_bond_attester() {
    let msg = RuntimeCall::AttesterIncentives(CallMessage::RegisterAttester(Amount::new(100)));
    let schema = Schema::of_single_type::<RuntimeCall>().unwrap();
    assert_eq!(
        schema.display(0, &borsh::to_vec(&msg).unwrap()).unwrap(),
        r#"AttesterIncentives.RegisterAttester(100)"#
    );
}
