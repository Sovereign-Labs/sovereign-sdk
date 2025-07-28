use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::runtime::TestRunner;
use sov_test_utils::{generate_optimistic_runtime, TestUser};
use sov_value_setter::{ValueSetter, ValueSetterConfig};

mod genesis;
mod multi_round;

mod dynamic_gas_update;

generate_optimistic_runtime!(TestChainStateRuntime <= value_setter: ValueSetter<S>);

type S = sov_test_utils::TestSpec;
type RT = TestChainStateRuntime<S>;

fn setup() -> (TestUser<S>, TestRunner<TestChainStateRuntime<S>, S>) {
    let genesis_config =
        HighLevelOptimisticGenesisConfig::generate().add_accounts_with_default_balance(1);

    let admin = genesis_config
        .additional_accounts()
        .first()
        .unwrap()
        .clone();

    let genesis = GenesisConfig::from_minimal_config(
        genesis_config.into(),
        ValueSetterConfig {
            admin: admin.address(),
        },
    );

    let runner = TestRunner::new_with_genesis(
        genesis.into_genesis_params(),
        TestChainStateRuntime::default(),
    );

    (admin, runner)
}
