//! These tests check that the gas price is increased/decreased correctly when the gas used
//! is above/below the gas target.

use sov_bank::Coins;
use sov_modules_api::macros::config_value;
use sov_modules_api::{Amount, Gas, GasArray, GasSpec, Spec};
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::runtime::genesis::TestTokenName;
use sov_test_utils::runtime::{Bank, TestRunner};
use sov_test_utils::{AsUser, TransactionTestCase, UserTokenInfo};
use sov_value_setter::{ValueSetter, ValueSetterConfig};

use crate::{GenesisConfig, TestChainStateRuntime, TestUser, RT, S};

struct TestData<S: Spec> {
    pub gas_target: S::Gas,
    pub token_name: TestTokenName,
    pub user: TestUser<S>,
}

/// To be able to test the dynamic gas price update we have to setup transactions that
/// would consume a high amount of gas. To do that we change the gas to charge in the bank
/// module for each call message.
/// The mint and burn calls are used to test the dynamic gas
/// price update. The mint call charges a very high gas amount (above the gas target) and
/// the burn call charges a very low gas amount (below the gas target).
fn setup_dynamic_gas_update_tests() -> (TestData<S>, TestRunner<TestChainStateRuntime<S>, S>) {
    let token_name = TestTokenName::new("TestToken".to_string());

    let genesis_config =
        HighLevelOptimisticGenesisConfig::generate().add_accounts(vec![TestUser::<S>::generate(
            Amount::MAX.saturating_div(Amount::new(2)),
        )
        .add_token_info(UserTokenInfo {
            token_name: token_name.clone(),
            balance: Amount::ZERO,
            is_minter: true,
        })]);

    let user = genesis_config
        .additional_accounts()
        .first()
        .unwrap()
        .clone();

    let genesis = GenesisConfig::from_minimal_config(
        genesis_config.into(),
        ValueSetterConfig {
            admin: user.address(),
        },
    );

    let mut gas_limit = <S as Spec>::Gas::from(config_value!("INITIAL_GAS_LIMIT"));
    let gas_target = gas_limit.scalar_division(2);

    let runtime = TestChainStateRuntime::<S>::default();

    let runner = TestRunner::new_with_genesis(genesis.into_genesis_params(), runtime);

    (
        TestData {
            gas_target: gas_target.clone(),
            token_name,
            user,
        },
        runner,
    )
}

#[test]
fn test_gas_price_increases_if_gas_used_exceeds_gas_target() {
    let (
        TestData {
            gas_target, user, ..
        },
        mut runner,
    ) = setup_dynamic_gas_update_tests();

    runner.execute_transaction(TransactionTestCase {
        input: user
            .create_plain_message::<RT, ValueSetter<S>>(sov_value_setter::CallMessage::SetValue {
                value: 1,
                gas: Some(gas_target.clone()),
            })
            .with_max_fee(Amount::from(u64::MAX / 2)),
        assert: Box::new(move |result, _| {
            assert!(result.tx_receipt.is_successful());

            assert!(
                result.gas_value_used > gas_target.value(&S::initial_base_fee_per_gas()).0,
                "The gas used should be greater than the gas target"
            );
        }),
    });

    let result = runner.execute(user.create_plain_message::<RT, ValueSetter<S>>(
        sov_value_setter::CallMessage::SetValue {
            value: 10,
            gas: None,
        },
    ));

    assert_eq!(result.0.batch_receipts.len(), 1);
    let gas_price = result.0.batch_receipts[0].inner.gas_price.clone();

    let initial_gas_price = S::initial_base_fee_per_gas();

    assert!(
        initial_gas_price.dim_is_less_than(&gas_price),
        "The gas price should have increased, current gas price: {gas_price:?}, initial gas price: {initial_gas_price:?}"
    );
}

#[test]
fn test_gas_price_decreases_if_gas_used_is_below_gas_target() {
    let (
        TestData {
            gas_target,
            token_name,
            user,
        },
        mut runner,
    ) = setup_dynamic_gas_update_tests();

    runner.execute_transaction(TransactionTestCase {
        input: user
            .create_plain_message::<RT, Bank<S>>(sov_bank::CallMessage::Burn {
                coins: Coins {
                    amount: Amount::ZERO,
                    token_id: token_name.id(),
                },
            })
            .with_max_fee(Amount::from(u64::MAX / 2)),
        assert: Box::new(move |result, _| {
            assert!(result.tx_receipt.is_successful());

            assert!(
                result.gas_value_used < gas_target.value(&S::initial_base_fee_per_gas()).0,
                "The gas used should be lower than the gas target"
            );
        }),
    });

    let result = runner.execute(user.create_plain_message::<RT, ValueSetter<S>>(
        sov_value_setter::CallMessage::SetValue {
            value: 10,
            gas: None,
        },
    ));

    assert_eq!(result.0.batch_receipts.len(), 1);
    let gas_price = result.0.batch_receipts[0].inner.gas_price.clone();

    let initial_gas_price = S::initial_base_fee_per_gas();

    assert!(
        gas_price.dim_is_less_than(&initial_gas_price),
        "The gas price should have decreased, current gas price: {gas_price:?}, initial gas price: {initial_gas_price:?}"
    );
}
