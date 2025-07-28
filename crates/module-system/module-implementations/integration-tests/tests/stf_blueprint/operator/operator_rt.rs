use sov_test_utils::runtime::genesis::operator::HighLevelOperatorGenesisConfig;
use sov_test_utils::runtime::TestRunner;
use sov_test_utils::{generate_operator_runtime, TestSequencer, TestUser};
use sov_value_setter::ValueSetter;

use crate::stf_blueprint::S;

generate_operator_runtime!(IntegTestRuntime <= value_setter: ValueSetter<S>);

#[allow(clippy::type_complexity)]
pub fn setup(
    reward_user: TestUser<S>,
    nb_of_users: usize,
) -> (
    TestRunner<IntegTestRuntime<S>, S>,
    Vec<TestUser<S>>,
    TestSequencer<S>,
) {
    let genesis_config = HighLevelOperatorGenesisConfig::<S>::generate_with_additional_accounts(
        nb_of_users,
        reward_user,
    );

    let admin = genesis_config.additional_accounts()[0].address();

    let genesis = GenesisConfig::from_minimal_config(
        genesis_config.clone().into(),
        sov_value_setter::ValueSetterConfig { admin },
    );

    let runner: TestRunner<IntegTestRuntime<S>, S> =
        TestRunner::new_with_genesis(genesis.into_genesis_params(), Default::default());

    let sequencer_account = genesis_config.initial_sequencer.clone();

    (
        runner,
        genesis_config.additional_accounts().clone(),
        sequencer_account,
    )
}
