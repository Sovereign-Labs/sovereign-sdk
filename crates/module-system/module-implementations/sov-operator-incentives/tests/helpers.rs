use sov_test_utils::runtime::genesis::operator::HighLevelOperatorGenesisConfig;
use sov_test_utils::runtime::TestRunner;
use sov_test_utils::{generate_operator_runtime, TestUser};

pub type S = sov_test_utils::TestSpec;
generate_operator_runtime!(TestRuntime <= );

pub type RT = TestRuntime<S>;

pub fn setup(reward_user: TestUser<S>) -> TestRunner<RT, S> {
    let genesis_config =
        HighLevelOperatorGenesisConfig::<S>::generate_with_additional_accounts(0, reward_user);

    let genesis = GenesisConfig::from_minimal_config(genesis_config.clone().into());

    let runner: TestRunner<RT, S> =
        TestRunner::new_with_genesis(genesis.into_genesis_params(), Default::default());

    runner
}
