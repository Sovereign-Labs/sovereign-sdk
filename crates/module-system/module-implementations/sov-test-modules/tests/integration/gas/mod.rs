use sov_modules_api::macros::config_value;
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::{Amount, Error, Gas, GasSpec, Spec, TxEffect};
use sov_test_modules::gas::{CallMessage, GasTester};
use sov_test_utils::{
    AsUser, AtomicAmount, TransactionTestAssert, TransactionTestCase, TEST_DEFAULT_USER_BALANCE,
};
mod helpers;
use helpers::{setup, RT, S};
use sov_bank::{config_gas_token_id, Bank};

/// Additional context for the post-call assertions of the gas tests.
/// This is used to check the outcome of the create token call.
struct PostSetValueContext {
    user_initial_balance: Amount,
    user_address: <S as Spec>::Address,
}

/// A helper function that creates a token with a custom gas config and checks the balance of the user
/// after the call.
/// The gas config can be set to `None` to use the default gas config.
fn gas_test_setup(
    set_value_assert: impl FnOnce(PostSetValueContext) -> TransactionTestAssert<RT, S> + 'static,
) {
    let (user, mut runner) = setup();

    let user_balance = user.available_gas_balance;

    runner.execute_transaction(TransactionTestCase {
        input: user.create_plain_message::<RT, GasTester<S>>(CallMessage::SetValue { value: 1 }),
        assert: Box::new(move |result, state| {
            set_value_assert(PostSetValueContext {
                user_initial_balance: user_balance,
                user_address: user.address(),
            })(result, state);
        }),
    });
}

/// Test that the gas price constants are charged correctly for the bank runtime.
/// To do that we override the gas config, set the costs to zero, execute a call and store the gas consumed.
/// Then we try with a different runtime config and check that the gas consumed only increases by the amount specified in the second config.
#[test]
fn gas_price_constants_are_charged_correctly() {
    let gas_consumed_without_price_ref = AtomicAmount::new(Amount::ZERO);
    let gas_consumed_without_price_ref_1 = gas_consumed_without_price_ref.clone();
    std::env::set_var("SOV_TEST_CONST_OVERRIDE_EXAMPLE_CUSTOM_GAS_PRICE", "[0, 0]");

    gas_test_setup(
        move |PostSetValueContext {
                  user_address,
                  user_initial_balance,
              }| {
            Box::new(move |result, state| {
                assert!(result.tx_receipt.is_successful());

                let user_final_balance = Bank::<S>::default()
                    .get_balance_of(&user_address, config_gas_token_id(), state)
                    .unwrap_infallible()
                    .unwrap();

                assert_eq!(
                    user_final_balance,
                    user_initial_balance
                        .checked_sub(result.gas_value_used)
                        .unwrap(),
                    "the balance should decrease only by the gas used"
                );

                gas_consumed_without_price_ref.add(result.gas_value_used);
            })
        },
    );

    let gas_charge_for_set_value = <S as Spec>::Gas::from([100; 2]);
    std::env::set_var(
        "SOV_TEST_CONST_OVERRIDE_EXAMPLE_CUSTOM_GAS_PRICE",
        "[100, 100]",
    );
    let bank_initial_gas_price = S::initial_base_fee_per_gas();

    gas_test_setup(
        move |PostSetValueContext {
                  user_initial_balance,
                  user_address,
              }| {
            Box::new(move |result, state| {
                assert!(result.tx_receipt.is_successful());

                let user_final_balance = Bank::<S>::default()
                    .get_balance_of(&user_address, config_gas_token_id(), state)
                    .unwrap_infallible()
                    .unwrap();

                assert_eq!(
                    user_final_balance,
                    user_initial_balance
                        .checked_sub(result.gas_value_used)
                        .unwrap(),
                    "the balance should decrease only by the gas used"
                );

                assert_eq!(
                gas_consumed_without_price_ref_1.get().checked_add(gas_charge_for_set_value.value(&bank_initial_gas_price)).unwrap(),
                result.gas_value_used,
                "The gas used should be the sum of the gas cost of the call and the inner gas cost"
            );
            })
        },
    );
}

#[test]
fn config_constants_are_charged_correctly() {
    let gas_consumed_without_price_ref = AtomicAmount::new(Amount::ZERO);
    let gas_consumed_without_price_ref_1 = gas_consumed_without_price_ref.clone();

    let create_token_config_cost =
        <S as Spec>::Gas::from(config_value!("EXAMPLE_CUSTOM_GAS_PRICE"));
    let bank_initial_gas_price = S::initial_base_fee_per_gas();

    gas_test_setup(
        move |PostSetValueContext {
                  user_initial_balance,
                  user_address,
              }| {
            Box::new(move |result, state| {
                assert!(result.tx_receipt.is_successful());

                let user_final_balance = Bank::<S>::default()
                    .get_balance_of(&user_address, config_gas_token_id(), state)
                    .unwrap_infallible()
                    .unwrap();

                assert_eq!(
                    user_final_balance,
                    user_initial_balance
                        .checked_sub(result.gas_value_used)
                        .unwrap(),
                    "the balance should be unchanged with zeroed price"
                );

                gas_consumed_without_price_ref.add(result.gas_value_used);
            })
        },
    );

    std::env::set_var("SOV_TEST_CONST_OVERRIDE_EXAMPLE_CUSTOM_GAS_PRICE", "[0, 0]");
    gas_test_setup(
        move |PostSetValueContext {
                  user_initial_balance,
                  user_address,
              }| {
            Box::new(move |result, state| {
                assert!(result.tx_receipt.is_successful());

                let user_final_balance = Bank::<S>::default()
                    .get_balance_of(&user_address, config_gas_token_id(), state)
                    .unwrap_infallible()
                    .unwrap();

                assert_eq!(
                    user_final_balance,
                    user_initial_balance
                        .checked_sub(result.gas_value_used)
                        .unwrap(),
                    "the balance should be unchanged with zeroed price"
                );

                assert_eq!(
                    gas_consumed_without_price_ref_1.get().checked_sub(create_token_config_cost.value(&bank_initial_gas_price)).unwrap(),
                    result.gas_value_used,
                    "The gas used should be the same as the gas consumed from the first call, minus the custom charge for the operation that we've removed"
                );
            })
        },
    );
}

#[test]
fn not_enough_gas_wont_panic() {
    let default_user_balance_as_u64: u64 = TEST_DEFAULT_USER_BALANCE.0
        .try_into()
        .expect("This test relies on setting the gas usage to half of the sender balance, but gas is only a u64. Lower the sender balance or update the test.");
    std::env::set_var(
        "SOV_TEST_CONST_OVERRIDE_EXAMPLE_CUSTOM_GAS_PRICE",
        format!(
            "[{}, {}]",
            default_user_balance_as_u64 / 2,
            default_user_balance_as_u64 / 2
        ),
    );
    gas_test_setup(|_| {
        Box::new(move |result, _state| {
            assert!(
                matches!(result.tx_receipt, TxEffect::Reverted(..)),
                "The transaction outcome is incorrect"
            );

            if let TxEffect::Reverted(contents) = result.tx_receipt {
                let Error::ModuleError(err) = contents.reason;
                let mut chain = err.chain();
                assert_eq!(chain.len(), 1, "The error chain is incorrect");

                assert!(
                    chain.next().unwrap().to_string().contains(
                        "The amount to charge is greater than the funds available in the meter."
                    ),
                    "The error message is incorrect"
                );
            } else {
                panic!("The transaction outcome is incorrect")
            }
        })
    });
}

#[test]
#[ignore = "This test is disabled because it we can't make gas charges overflow without increasing the base gas price beyond i64::MAX - which is currently unsupported by the toml crate. We'll need a way to do this manually for the test."]
fn very_high_gas_to_charge_should_overflow() {
    std::env::set_var(
        "SOV_TEST_CONST_OVERRIDE_EXAMPLE_CUSTOM_GAS_PRICE",
        format!("[{}, {}]", u64::MAX, u64::MAX),
    );
    gas_test_setup(|_| {
        Box::new(move |result, _state| {
            assert!(
                matches!(result.tx_receipt, TxEffect::Reverted(..)),
                "The transaction outcome is incorrect"
            );

            if let TxEffect::Reverted(contents) = result.tx_receipt {
                let Error::ModuleError(err) = contents.reason;
                let mut chain = err.chain();
                assert_eq!(chain.len(), 1, "The error chain is incorrect");

                assert!(
                        chain.next().unwrap().to_string().contains(
                            "Gas calculation overflow: Charge Funds: Unable to charge gas, because the calculation overflows"
                        ),
                        "The error message is incorrect"
                    );
            } else {
                panic!("The transaction outcome is incorrect")
            }
        })
    });
}
