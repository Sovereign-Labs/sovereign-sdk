use sov_modules_api::macros::UniversalWallet;
use sov_modules_api::sov_universal_wallet::schema::Schema;
use sov_modules_api::Error::ModuleError;
use sov_test_utils::runtime::genesis::zk::config::HighLevelZkGenesisConfig;
use sov_test_utils::runtime::{TestRunner, ValueSetter};
use sov_test_utils::{generate_zk_runtime, AsUser, TestSpec, TestUser, TransactionTestCase};
use sov_value_setter::{CallMessage, Event, SetValueError};

generate_zk_runtime!(TestRuntime <= value_setter: ValueSetter<S>);

type S = TestSpec;
type RT = TestRuntime<S>;

#[allow(clippy::type_complexity)]
fn setup() -> (TestRunner<TestRuntime<S>, S>, TestUser<S>, TestUser<S>) {
    let genesis_config = HighLevelZkGenesisConfig::generate_with_additional_accounts(2);

    let admin_account = genesis_config.additional_accounts()[0].clone();
    let extra_account = genesis_config.additional_accounts()[1].clone();

    let genesis = GenesisConfig::from_minimal_config(
        genesis_config.clone().into(),
        sov_value_setter::ValueSetterConfig {
            admin: admin_account.address(),
        },
    );

    (
        TestRunner::new_with_genesis(genesis.into_genesis_params(), Default::default()),
        admin_account,
        extra_account,
    )
}

#[test]
fn test_setting_value() {
    let (mut runner, admin, _) = setup();

    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, ValueSetter<S>>(CallMessage::SetValue {
            value: 5,
            gas: None,
        }),
        assert: Box::new(|result, state| {
            assert_eq!(
                ValueSetter::<S>::default().value.get(state).unwrap(),
                Some(5)
            );
            assert!(result.events.iter().any(|event| matches!(
                event,
                TestRuntimeEvent::ValueSetter(
                    Event::NewValue(value)
                ) if *value == 5
            )));
        }),
    });
}

#[test]
fn test_setting_value_not_admin() {
    let (mut runner, admin, non_admin) = setup();

    runner.execute_transaction(TransactionTestCase {
        input: non_admin.create_plain_message::<RT, ValueSetter<S>>(CallMessage::SetValue {
            value: 5,
            gas: None,
        }),
        assert: Box::new(move |result, _state| {
            match &result.tx_receipt {
                sov_modules_api::TxEffect::Reverted(reason) => {
                    assert_eq!(
                        &reason.reason,
                        &ModuleError(
                            SetValueError::<S>::WrongSender {
                                sender: non_admin.address(),
                                admin: admin.address()
                            }
                            .into()
                        ),
                        "Transaction reverted, but with unexpected reason"
                    );
                }
                unexpected => panic!("Expected transaction to revert, but got: {unexpected:?}"),
            };
        }),
    });
}

#[test]
fn test_display_value_setter_call() {
    #[derive(Debug, Clone, PartialEq, borsh::BorshSerialize, UniversalWallet)]
    enum RuntimeCall {
        ValueSetter(CallMessage<S>),
    }

    let msg = RuntimeCall::ValueSetter(CallMessage::SetValue {
        value: 92,
        gas: None,
    });

    let schema = Schema::of_single_type::<RuntimeCall>().unwrap();
    assert_eq!(
        schema.display(0, &borsh::to_vec(&msg).unwrap()).unwrap(),
        r#"ValueSetter.SetValue { value: 92, gas: None }"#
    );
}
