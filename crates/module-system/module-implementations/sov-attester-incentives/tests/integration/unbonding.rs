use sov_attester_incentives::{CustomError, UnbondingInfo};
use sov_mock_da::MockAddress;
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::registration_lib::RegistrationError;
use sov_modules_api::{Amount, Spec, StateAccessorError};
use sov_rollup_interface::common::SlotNumber;
use sov_test_utils::runtime::TestRunner;
use sov_test_utils::{
    AsUser, AtomicAmount, TestAttester, TransactionTestCase, TEST_LIGHT_CLIENT_FINALIZED_HEIGHT,
    TEST_ROLLUP_FINALITY_PERIOD,
};

use crate::helpers::{setup, TestAttesterIncentives, TestRuntimeEvent, RT, S};

const INIT_BONDING_HEIGHT: SlotNumber = TEST_LIGHT_CLIENT_FINALIZED_HEIGHT;

/// Checks that the attester is bonded and starts the unbonding process.
/// Returns the gas consumed by the attester when submitting the unbonding transaction.
fn check_attester_bonded_and_start_unbond(
    runner: &mut TestRunner<RT, S>,
    attester: &TestAttester<S>,
) -> Amount {
    let attester_address = attester.user_info.address();
    let attester_bond = attester.bond;
    let attester_balance = attester.user_info.balance();

    let gas_consumed_attester_ref_1 = AtomicAmount::new(Amount::ZERO);
    let gas_consumed_attester_ref_2 = gas_consumed_attester_ref_1.clone();

    runner.query_visible_state(|state| {
        assert_eq!(
            TestAttesterIncentives::default()
                .bonded_attesters
                .get(&attester_address, state)
                .unwrap(),
            Some(attester_bond),
            "The genesis attester should be bonded"
        );

        // Check the balance of the attester is equal to the free balance
        assert_eq!(
            TestRunner::<RT, S>::bank_gas_balance(&attester_address, state),
            Some(attester_balance),
            "The balance of the attester should be equal to the free balance"
        );
    });

    runner.execute_transaction(TransactionTestCase {
        input: attester.create_plain_message::<RT, TestAttesterIncentives>(
            sov_attester_incentives::CallMessage::BeginExitAttester,
        ),
        assert: Box::new(move |result, state| {
            assert_eq!(
                TestAttesterIncentives::default()
                    .unbonding_attesters
                    .get(&attester_address, state)
                    .unwrap(),
                Some(UnbondingInfo {
                    unbonding_initiated_height: INIT_BONDING_HEIGHT,
                    amount: attester_bond
                }),
            );
            gas_consumed_attester_ref_1.add(result.gas_value_used);
        }),
    });

    gas_consumed_attester_ref_2.get()
}

#[test]
fn try_unbond_successful() {
    let (mut runner, attester, _, _) = setup();

    let attester_address = attester.user_info.address();
    let attester_bond = attester.bond;
    let attester_balance = attester.user_info.balance();

    let gas_consumed_start_unbonding =
        check_attester_bonded_and_start_unbond(&mut runner, &attester);

    // Execute empty slots to finalize unbonding. Artificially increase the light client finalized height.
    runner.__apply_to_state(|state| {
        // Increase the light client finalized height
        TestAttesterIncentives::default()
            .light_client_finalized_height
            .set(
                &INIT_BONDING_HEIGHT.saturating_add(TEST_ROLLUP_FINALITY_PERIOD),
                state,
            )
            .unwrap_infallible();
    });

    runner.execute_transaction(TransactionTestCase {
        input: attester.create_plain_message::<RT, TestAttesterIncentives>(
            sov_attester_incentives::CallMessage::ExitAttester,
        ),
        assert: Box::new(move |result, state| {
            // Test that the unbonding attester event is emitted
            assert!(result.events.iter().any(|event| {
                event
                    == &TestRuntimeEvent::AttesterIncentives(
                        sov_attester_incentives::Event::ExitedAttester {
                            amount_withdrawn: attester_bond,
                        },
                    )
            }));

            assert_eq!(
                TestAttesterIncentives::default()
                    .unbonding_attesters
                    .get(&attester_address, state)
                    .unwrap_infallible(),
                None,
                "The attester should not be part of the unbonding set anymore"
            );

            assert_eq!(
                TestAttesterIncentives::default()
                    .bonded_attesters
                    .get(&attester_address, state)
                    .unwrap_infallible(),
                None,
                "The attester should not be part of the bonded set anymore"
            );

            // Check the final balance of the attester
            assert_eq!(
                TestRunner::<RT, S>::bank_gas_balance(&attester_address, state),
                Some(
                    attester_balance
                        .checked_add(attester_bond)
                        .unwrap()
                        .checked_sub(result.gas_value_used)
                        .unwrap()
                        .checked_sub(gas_consumed_start_unbonding)
                        .unwrap()
                )
            );
        }),
    });
}

#[test]
fn try_unbond_too_early() {
    let (mut runner, attester, _, _) = setup();
    let addr = attester.as_user().address();
    check_attester_bonded_and_start_unbond(&mut runner, &attester);

    // Finalize unbonding, this should fail because the unbonding period has not passed yet
    runner.execute_transaction(TransactionTestCase {
        input: attester.create_plain_message::<RT, TestAttesterIncentives>(
            sov_attester_incentives::CallMessage::ExitAttester,
        ),
        assert: Box::new(move |result, _state| {
            match &result.tx_receipt {
                sov_modules_api::TxEffect::Reverted(reason) => {
                    assert_eq!(
                        reason.reason.to_string(),
                        RegistrationError::<
                            MockAddress,
                            MockAddress,
                            StateAccessorError<<S as Spec>::Gas>,
                            _,
                        >::Custom(CustomError::UnbondingNotFinalized(addr))
                        .to_string(),
                        "Transaction reverted, but with unexpected reason"
                    );
                }
                unexpected => panic!("Expected transaction to revert, but got: {unexpected:?}"),
            };
        }),
    });
}

/// The attester tries to unbond without bonding
#[test]
fn try_unbond_without_bonding() {
    let (mut runner, _, _, additional_account) = setup();

    let additional_account_address = additional_account.address();

    runner.query_visible_state(|state| {
        // Check that the additional account is not bonded

        assert_eq!(
            TestAttesterIncentives::default()
                .bonded_attesters
                .get(&additional_account_address, state)
                .unwrap(),
            None,
            "The additional account should not be bonded"
        );
    });

    runner.execute_transaction(TransactionTestCase {
        input: additional_account.create_plain_message::<RT, TestAttesterIncentives>(
            sov_attester_incentives::CallMessage::BeginExitAttester,
        ),
        assert: Box::new(move |_result, state| {
            assert_eq!(
                TestAttesterIncentives::default()
                    .unbonding_attesters
                    .get(&additional_account_address, state)
                    .unwrap(),
                None,
                "The additional account should not be part of the unbonding set"
            );
        }),
    });
}

/// The attester tries to unbond without waiting for the two-phase unbonding to finalize
#[test]
fn try_skip_two_phase_unbonding() {
    let (mut runner, attester, _, _) = setup();
    let addr = attester.as_user().address();

    runner.execute_transaction(TransactionTestCase {
        input: attester.create_plain_message::<RT, TestAttesterIncentives>(
            sov_attester_incentives::CallMessage::ExitAttester,
        ),
        assert: Box::new(move |result, _state| {
            match &result.tx_receipt {
                sov_modules_api::TxEffect::Reverted(reason) => {
                    assert_eq!(
                        reason.reason.to_string(),
                        RegistrationError::<
                            MockAddress,
                            MockAddress,
                            StateAccessorError<<S as Spec>::Gas>,
                            _,
                        >::Custom(CustomError::AttesterIsNotUnbonding(
                            addr
                        ))
                        .to_string(),
                        "Transaction reverted, but with unexpected reason"
                    );
                }
                unexpected => panic!("Expected transaction to revert, but got: {unexpected:?}"),
            };
        }),
    });
}

/// The attester tries to bond while unbonding
#[test]
fn try_bond_while_unbonding() {
    let (mut runner, attester, _, _) = setup();
    let attester_address = attester.user_info.address();
    let attester_bond = attester.bond;

    let start_unbonding = TransactionTestCase {
        input: attester.create_plain_message::<RT, TestAttesterIncentives>(
            sov_attester_incentives::CallMessage::BeginExitAttester,
        ),
        assert: Box::new(move |_result, state| {
            // Check that the state has been updated correctly
            assert_eq!(
                TestAttesterIncentives::default()
                    .unbonding_attesters
                    .get(&attester_address, state)
                    .unwrap(),
                Some(UnbondingInfo {
                    unbonding_initiated_height: SlotNumber::GENESIS,
                    amount: attester_bond
                }),
                "The attester should be part of the unbonding set"
            );

            assert_eq!(
                TestAttesterIncentives::default()
                    .bonded_attesters
                    .get(&attester_address, state)
                    .unwrap(),
                None,
                "The attester should not be bonded"
            );
        }),
    };
    let try_bond = TransactionTestCase {
        input: attester.create_plain_message::<RT, TestAttesterIncentives>(
            sov_attester_incentives::CallMessage::RegisterAttester(Amount::new(100)),
        ),
        assert: Box::new(move |result, _state| {
            match &result.tx_receipt {
                sov_modules_api::TxEffect::Reverted(reason) => {
                    assert_eq!(
                        reason.reason.to_string(),
                        RegistrationError::<
                            MockAddress,
                            MockAddress,
                            StateAccessorError<<S as Spec>::Gas>,
                            _,
                        >::Custom(CustomError::AttesterIsUnbonding(
                            attester_address
                        ))
                        .to_string(),
                        "Transaction reverted, but with unexpected reason"
                    );
                }
                unexpected => panic!("Expected transaction to revert, but got: {unexpected:?}"),
            };
        }),
    };

    runner
        // The attester starts unbonding
        .execute_transaction(start_unbonding)
        // The attester shouldn't be able to bond while unbonding
        .execute_transaction(try_bond);
}
