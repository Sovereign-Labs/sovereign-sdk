use sov_modules_api::macros::UniversalWallet;
use sov_modules_api::sov_universal_wallet::schema::Schema;
use sov_modules_api::Amount;
use sov_prover_incentives::CallMessage;

#[derive(Debug, PartialEq, borsh::BorshSerialize, UniversalWallet)]
enum RuntimeCall {
    ProverIncentives(CallMessage),
}

#[test]
fn test_display_accounts_call() {
    let msg = RuntimeCall::ProverIncentives(CallMessage::Register(Amount::new(100)));

    let schema = Schema::of_single_type::<RuntimeCall>().unwrap();
    assert_eq!(
        schema.display(0, &borsh::to_vec(&msg).unwrap()).unwrap(),
        r#"ProverIncentives.Register(100)"#
    );
}
