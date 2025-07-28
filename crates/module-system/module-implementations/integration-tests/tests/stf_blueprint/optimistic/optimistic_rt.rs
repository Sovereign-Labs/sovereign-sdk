use sov_modules_api::ApiStateAccessor;
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::runtime::TestRunner;
use sov_test_utils::{
    generate_optimistic_runtime, TestSequencer, TestUser, TEST_DEFAULT_USER_BALANCE,
};
use sov_value_setter::ValueSetter;

use crate::stf_blueprint::{S, *};

type RT = IntegTestRuntime<S>;

generate_optimistic_runtime!(IntegTestRuntime <= value_setter: ValueSetter<S>);

#[allow(clippy::type_complexity)]
pub fn setup(
    nb_of_users: usize,
) -> (
    TestRunner<IntegTestRuntime<S>, S>,
    Vec<TestUser<S>>,
    TestSequencer<S>,
) {
    let mut genesis_config = HighLevelOptimisticGenesisConfig::generate();

    for _ in 0..nb_of_users {
        genesis_config
            .additional_accounts_mut()
            .push(TestUser::<S>::generate(TEST_DEFAULT_USER_BALANCE));
    }
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

pub mod helpers {
    use sov_attester_incentives::AttesterIncentives;
    use sov_bank::IntoPayable;
    use sov_modules_api::{Amount, ModuleInfo};
    use sov_test_utils::{TestSequencer, TestUser};

    use super::*;

    pub(crate) struct Actors {
        pub(crate) admin_account: TestUser<S>,
        pub(crate) not_admin_account: TestUser<S>,
        pub(crate) sequencer_account: TestSequencer<S>,
    }

    impl Actors {
        pub(crate) fn balances(&self, state: &mut ApiStateAccessor<S>) -> Balances {
            let attester_module = AttesterIncentives::<S>::default();
            Balances {
                admin_balance: get_balance(&self.admin_account.address(), state),
                not_admin_balance: get_balance(&self.not_admin_account.address(), state),
                attester_module_balance: get_balance(attester_module.id().to_payable(), state),
                sequencer_bond: get_seq_bond(&self.sequencer_account.da_address, state).unwrap(),
            }
        }
    }

    #[derive(Debug, Eq, PartialEq)]
    pub(crate) struct Balances {
        pub(crate) admin_balance: Amount,
        pub(crate) not_admin_balance: Amount,
        pub(crate) attester_module_balance: Amount,
        pub(crate) sequencer_bond: Amount,
    }

    impl Balances {
        pub(crate) fn total_balance(&self) -> Amount {
            self.admin_balance
                .checked_add(self.not_admin_balance)
                .unwrap()
                .checked_add(self.sequencer_bond)
                .unwrap()
                .checked_add(self.attester_module_balance)
                .unwrap()
        }
    }
}
