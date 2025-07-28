use sov_mock_da::MockAddress;
use sov_modules_api::macros::UniversalWallet;
use sov_modules_api::sov_universal_wallet::schema::Schema;
use sov_modules_api::Amount;
use sov_sequencer_registry::CallMessage;
use sov_test_utils::TestSpec;

type S = TestSpec;

#[derive(Debug, PartialEq, borsh::BorshSerialize, UniversalWallet)]
enum RuntimeCall {
    SequencerRegistry(CallMessage<S>),
}

#[test]
fn test_display_sequencer_registry_call() {
    let msg = RuntimeCall::SequencerRegistry(CallMessage::Deposit {
        da_address: MockAddress::new([1; 32]),
        amount: Amount::new(100),
    });

    let schema = Schema::of_single_type::<RuntimeCall>().unwrap();
    assert_eq!(
        schema.display(0, &borsh::to_vec(&msg).unwrap()).unwrap(),
        "SequencerRegistry.Deposit { da_address: 0x0101010101010101010101010101010101010101010101010101010101010101, amount: 100 }"
    );
}
