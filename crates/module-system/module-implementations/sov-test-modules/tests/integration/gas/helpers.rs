use sov_modules_api::Amount;
use sov_test_modules::gas::GasTester;
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::runtime::genesis::TestTokenName;
use sov_test_utils::runtime::TestRunner;
use sov_test_utils::{generate_optimistic_runtime, TestUser};

pub type S = sov_test_utils::TestSpec;

generate_optimistic_runtime!(TestGasRuntime <= gas_tester: GasTester<S>);

/// The default runtime type used in the bank tests.
pub type RT = TestGasRuntime<S>;

/// Sets up the bank tests by generating a genesis config with a single non-default token that has
/// - a minter
/// - a user with a high token balance
/// - a user with no balance for the token
///
/// Also allows to set up a custom runtime using a closure. Useful for the gas tests to change the gas costs
/// of the bank runtime.
pub fn setup() -> (TestUser<S>, TestRunner<RT, S>) {
    let token_name = TestTokenName::new("BankToken".to_string());

    let genesis_config = HighLevelOptimisticGenesisConfig::generate()
        .add_accounts_with_default_balance(1)
        .add_accounts_with_token(&token_name, true, 1, Amount::new(100_000));

    let user_no_token_balance = genesis_config.additional_accounts()[0].clone();

    assert!(user_no_token_balance.token_balance(&token_name).is_none());

    let mut token_users_vec = genesis_config.get_accounts_for_token(&token_name);

    let user_high_token_balance = token_users_vec.pop().unwrap();

    let genesis = GenesisConfig::from_minimal_config(genesis_config.into(), ());

    let runtime = RT::default();

    let runner = TestRunner::new_with_genesis(genesis.into_genesis_params(), runtime);

    (user_high_token_balance, runner)
}
