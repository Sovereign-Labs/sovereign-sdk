use module_template::{CallMessage, ExampleModule, ExampleModuleConfig, Response};
use sov_modules_api::macros::UniversalWallet;
use sov_modules_api::sov_universal_wallet::schema::Schema;
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::runtime::TestRunner;
use sov_test_utils::{generate_optimistic_runtime, AsUser, TransactionTestCase};

generate_optimistic_runtime!(ExampleModuleRuntime <= example_module: ExampleModule<S>);

type S = sov_test_utils::TestSpec;
type RT = ExampleModuleRuntime<S>;

#[test]
fn test_example_module() {
    // Generate a genesis config, then overwrite the attester key/address with ones that
    // we know. We leave the other values untouched.
    let genesis_config =
        HighLevelOptimisticGenesisConfig::generate().add_accounts_with_default_balance(1);

    let user = genesis_config
        .additional_accounts()
        .first()
        .unwrap()
        .clone();

    // Run genesis registering the attester and sequencer we've generated.
    let genesis = GenesisConfig::from_minimal_config(genesis_config.into(), ExampleModuleConfig {});

    let mut runner = TestRunner::<_, _>::new_with_genesis(
        genesis.into_genesis_params(),
        ExampleModuleRuntime::default(),
    );

    runner.execute_transaction(TransactionTestCase {
        input: user.create_plain_message::<RT, ExampleModule<S>>(CallMessage::SetValue(99)),
        assert: Box::new(|result, _state| {
            assert!(result.tx_receipt.is_successful());

            assert_eq!(result.events.len(), 1);
            assert_eq!(
                result.events[0],
                ExampleModuleRuntimeEvent::ExampleModule(module_template::Event::Set { value: 99 })
            );
        }),
    });

    runner.query_visible_state(|state| {
        assert_eq!(
            ExampleModule::<S>::default().query_value(state),
            Response { value: Some(99) }
        );
    });
}

#[test]
fn test_display_value_setter_call() {
    #[derive(Debug, PartialEq, borsh::BorshSerialize, UniversalWallet)]
    enum RuntimeCall {
        ValueSetter(CallMessage),
    }

    let msg = RuntimeCall::ValueSetter(CallMessage::SetValue(5));

    let schema = Schema::of_single_type::<RuntimeCall>().unwrap();
    assert_eq!(
        schema.display(0, &borsh::to_vec(&msg).unwrap()).unwrap(),
        r#"ValueSetter.SetValue(5)"#
    );
}
