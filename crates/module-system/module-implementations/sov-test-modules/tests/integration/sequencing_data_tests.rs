use sov_test_modules::sequencing_data::SequencingDataTester;
use sov_test_utils::runtime::genesis::zk::config::HighLevelZkGenesisConfig;
use sov_test_utils::runtime::TestRunner;
use sov_test_utils::{generate_zk_runtime, AsUser, TestSpec, TestUser, TransactionTestCase};

generate_zk_runtime!(Runtime <= sequencing_data_tester: SequencingDataTester<S>);

type S = TestSpec;
type RT = Runtime<S>;

fn setup() -> (TestRunner<RT, S>, TestUser<S>) {
    let genesis_config = HighLevelZkGenesisConfig::generate_with_additional_accounts(1);
    let account = genesis_config.additional_accounts()[0].clone();

    let genesis = GenesisConfig::from_minimal_config(genesis_config.into(), ());
    let runner = TestRunner::new_with_genesis(genesis.into_genesis_params(), Default::default());
    (runner, account)
}

#[test]
fn success() {
    let (mut runner, user) = setup();

    runner.execute_transaction(TransactionTestCase {
        input: user.create_plain_message::<RT, SequencingDataTester<S>>(()),
        assert: Box::new(|result, _state| {
            assert!(
                result.tx_receipt.is_successful(),
                "Transaction should succeed. Receipt: {:?}",
                result.tx_receipt
            );
        }),
    });
}
