use sov_paymaster::{PaymasterConfig, SafeVec};
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::runtime::{GenesisConfig, TestOptimisticRuntime, TestRunner};
use sov_test_utils::{TestSpec, TestUser};
use sov_value_setter::ValueSetterConfig;

pub type S = TestSpec;
pub type RT = TestOptimisticRuntime<S>;

/// Sets up a test runner with the [`ValueSetter`] with a single additional admin account.
pub fn setup() -> (TestUser<S>, TestRunner<TestOptimisticRuntime<S>, S>) {
    let genesis_config =
        HighLevelOptimisticGenesisConfig::generate().add_accounts_with_default_balance(1);

    let admin = genesis_config
        .additional_accounts()
        .first()
        .unwrap()
        .clone();

    let value_setter_config = ValueSetterConfig {
        admin: admin.address(),
    };
    let paymaster_config = PaymasterConfig {
        payers: SafeVec::new(),
    };

    // Run genesis registering the attester and sequencer we've generated.
    let genesis = GenesisConfig::from_minimal_config(
        genesis_config.into(),
        value_setter_config,
        paymaster_config,
    );

    let runner = TestRunner::new_with_genesis(
        genesis.into_genesis_params(),
        TestOptimisticRuntime::default(),
    );

    (admin, runner)
}
