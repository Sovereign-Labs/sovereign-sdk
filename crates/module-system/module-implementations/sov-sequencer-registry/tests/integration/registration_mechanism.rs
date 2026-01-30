use sov_mock_da::MockAddress;
use sov_modules_api::macros::config_value;
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::transaction::PriorityFeeBips;
use sov_modules_api::{Amount, Gas, GasArray, GasSpec, GetGasPrice, TxEffect};
use sov_sequencer_registry::{CallMessage, CustomError};
use sov_test_utils::runtime::{TestRunner, ValueSetter};
use sov_test_utils::{
    AsUser, AtomicAmount, BatchTestCase, BatchType, TransactionTestCase, TEST_DEFAULT_USER_BALANCE,
};

use crate::helpers::{
    setup, TestRoles, TestRuntimeEvent, TestSequencerRegistry, TestSequencerRegistryError,
    ANOTHER_SEQUENCER_DA_ADDRESS, NON_DEFAULT_SEQUENCER_DA_ADDRESS, RT,
};

const SEQUENCE_STAKE: Amount = Amount::new(100_000_000_000);

type S = sov_test_utils::TestSpec;

// Happy path for registration and exit.
// This test checks:
//  - genesis sequencer is present after genesis
//  - he can process transactions
#[test]
fn test_default_sequencer() {
    let (
        TestRoles {
            default_sequencer: test_sequencer,
            admin,
            ..
        },
        mut runner,
    ) = setup();

    let test_sequencer_address = test_sequencer.user_info.address();
    let test_sequencer_da_address = test_sequencer.da_address;
    let test_sequencer_bond = test_sequencer.bond;

    let custom_priority_fee = PriorityFeeBips::from_percentage(10);

    runner.execute_transaction(TransactionTestCase {
        input: admin
            .create_plain_message::<RT, ValueSetter<S>>(sov_value_setter::CallMessage::SetValue {
                value: 10,
                gas: None,
            })
            .with_max_priority_fee_bips(custom_priority_fee),
        assert: Box::new(move |result, state| {
            // Assert that the sequencer has been rewarded
            let gas_price = state.gas_price();
            let sequencer_burn = S::gas_to_charge_per_byte_borsh_deserialization()
                .checked_scalar_product(result.blob_info.size as u64)
                .unwrap()
                .checked_value(gas_price)
                .unwrap();
            assert_eq!(
                TestRunner::<RT, S>::get_sequencer_staking_balance(
                    &test_sequencer_da_address,
                    state
                ),
                Some(
                    test_sequencer_bond
                        .checked_add(custom_priority_fee.apply(result.gas_value_used).unwrap())
                        .unwrap()
                        .checked_sub(sequencer_burn)
                        .unwrap()
                ),
                "The sequencer should have been rewarded the execution funds "
            );

            assert_eq!(
                TestSequencerRegistry::default()
                    .is_sender_known(&test_sequencer_da_address, state)
                    .unwrap()
                    .address,
                test_sequencer_address
            );
        }),
    });
}

#[test]
fn test_new_sequencer_registration() {
    let (
        TestRoles {
            additional_sequencer,
            admin,
            ..
        },
        mut runner,
    ) = setup();

    let other_sequencer_address = additional_sequencer.address();
    let other_sequencer_da_address = MockAddress::new(NON_DEFAULT_SEQUENCER_DA_ADDRESS);

    runner.execute_transaction(TransactionTestCase {
        input: additional_sequencer.create_plain_message::<RT, TestSequencerRegistry>(
            sov_sequencer_registry::CallMessage::Register {
                da_address: other_sequencer_da_address,
                amount: SEQUENCE_STAKE,
            },
        ),
        assert: Box::new(move |result, state| {
            // Assert that the sequencer has correctly been registered
            assert!(
                TestSequencerRegistry::default()
                    .is_sender_known(&MockAddress::new(NON_DEFAULT_SEQUENCER_DA_ADDRESS), state)
                    .is_ok(),
                "The sequencer is not registered"
            );
            // Assert that a registration event has been emitted
            assert!(result.events.iter().any(|event| matches!(
                event,
                TestRuntimeEvent::SequencerRegistry(
                    sov_sequencer_registry::Event::Registered { sequencer, amount }
                ) if *sequencer == other_sequencer_address && *amount == SEQUENCE_STAKE.0
            )));
            // Assert that the other sequencer balance has been updated
            assert_eq!(
                TestRunner::<RT, S>::bank_gas_balance(&other_sequencer_address, state),
                Some(
                    TEST_DEFAULT_USER_BALANCE
                        .checked_sub(SEQUENCE_STAKE)
                        .unwrap()
                        .checked_sub(result.gas_value_used)
                        .unwrap()
                )
            );
        }),
    });

    runner.config.sequencer_da_address = other_sequencer_da_address;

    runner.execute_batch(BatchTestCase {
        input: vec![admin.create_plain_message::<RT, ValueSetter<S>>(
            sov_value_setter::CallMessage::SetValue {
                value: 10,
                gas: None,
            },
        )]
        .into(),
        assert: Box::new(move |result, _state| {
            assert!(result.batch_receipt.unwrap().tx_receipts[0]
                .receipt
                .is_successful());
        }),
    });
}

#[test]
fn test_registration_bond_too_small() {
    let (
        TestRoles {
            additional_sequencer,
            ..
        },
        mut runner,
    ) = setup();

    let seq_address = additional_sequencer.address();
    let amount_to_register = Amount::new(100);

    runner.execute_transaction(TransactionTestCase {
        input: additional_sequencer.create_plain_message::<RT, TestSequencerRegistry>(
            sov_sequencer_registry::CallMessage::Register {
                da_address: NON_DEFAULT_SEQUENCER_DA_ADDRESS.into(),
                amount: amount_to_register,
            },
        ),
        assert: Box::new(move |result, _state| match &result.tx_receipt {
            TxEffect::Reverted(reason) => {
                assert_eq!(
                    reason.reason.to_string(),
                    TestSequencerRegistryError::InsufficientStakeAmount {
                        address: seq_address,
                        bond_amount: amount_to_register,
                        minimum_bond_amount: Amount::new(1000)
                    }
                    .to_string(),
                    "Transaction reverted, but with unexpected reason"
                );
            }
            unexpected => panic!("Expected transaction to revert, but got: {unexpected:?}"),
        }),
    });

    let amount_to_register = Amount::new(1000);

    runner.execute_transaction(TransactionTestCase {
        input: additional_sequencer.create_plain_message::<RT, TestSequencerRegistry>(
            sov_sequencer_registry::CallMessage::Register {
                da_address: NON_DEFAULT_SEQUENCER_DA_ADDRESS.into(),
                amount: amount_to_register,
            },
        ),
        assert: Box::new(move |result, _state| assert!(result.tx_receipt.is_successful())),
    });
}

#[test]
fn test_registration_not_enough_funds() {
    let (
        TestRoles {
            additional_sequencer,
            ..
        },
        mut runner,
    ) = setup();

    let other_sequencer_balance = additional_sequencer.available_gas_balance;

    let additional_sequencer_address = additional_sequencer.address();

    let amount_to_register = other_sequencer_balance.checked_add(SEQUENCE_STAKE).unwrap();

    runner.execute_transaction(TransactionTestCase {
        input: additional_sequencer.create_plain_message::<RT, TestSequencerRegistry>(
            sov_sequencer_registry::CallMessage::Register {
                da_address: NON_DEFAULT_SEQUENCER_DA_ADDRESS.into(),
                amount: amount_to_register,
            },
        ),
        assert: Box::new(move |result, _state| match &result.tx_receipt {
            TxEffect::Reverted(reason) => {
                assert_eq!(
                    reason.reason.to_string(),
                    TestSequencerRegistryError::InsufficientFundsToRegister {
                        address: additional_sequencer_address,
                        amount: amount_to_register,
                    }
                    .to_string(),
                    "Transaction reverted, but with unexpected reason"
                );
            }
            unexpected => panic!("Expected transaction to revert, but got: {unexpected:?}"),
        }),
    });
}

#[test]
fn test_registration_second_time() {
    let (
        TestRoles {
            additional_sequencer,
            ..
        },
        mut runner,
    ) = setup();

    let other_sequencer_address = additional_sequencer.address();

    runner.execute(
        additional_sequencer.create_plain_message::<RT, TestSequencerRegistry>(
            sov_sequencer_registry::CallMessage::Register {
                da_address: NON_DEFAULT_SEQUENCER_DA_ADDRESS.into(),
                amount: SEQUENCE_STAKE,
            },
        ),
    );

    runner.execute_transaction(TransactionTestCase {
        input: additional_sequencer.create_plain_message::<RT, TestSequencerRegistry>(
            sov_sequencer_registry::CallMessage::Register {
                da_address: NON_DEFAULT_SEQUENCER_DA_ADDRESS.into(),
                amount: SEQUENCE_STAKE,
            },
        ),
        assert: Box::new(move |result, _state| match &result.tx_receipt {
            TxEffect::Reverted(reason) => {
                assert_eq!(
                    reason.reason.to_string(),
                    TestSequencerRegistryError::AlreadyRegistered(other_sequencer_address)
                        .to_string(),
                    "Transaction reverted, but with unexpected reason"
                );
            }
            unexpected => panic!("Expected transaction to revert, but got: {unexpected:?}"),
        }),
    });
}

/// Tests that another sequencer can register and exit.
#[test]
fn test_exit_happy_path() {
    let (roles, mut runner) = setup();

    let additional_sequencer = roles.additional_sequencer;

    let other_sequencer_address = additional_sequencer.address();
    let other_sequencer_da_address = MockAddress::new(NON_DEFAULT_SEQUENCER_DA_ADDRESS);

    let other_sequencer_balance_ref = AtomicAmount::new(additional_sequencer.available_gas_balance);
    let other_sequencer_balance_ref_1 = other_sequencer_balance_ref.clone();
    let other_sequencer_balance_ref_2 = other_sequencer_balance_ref.clone();
    let other_sequencer_balance_ref_3 = other_sequencer_balance_ref.clone();
    let other_sequencer_balance_ref_4 = other_sequencer_balance_ref.clone();

    let register = TransactionTestCase {
        input: additional_sequencer.create_plain_message::<RT, TestSequencerRegistry>(
            sov_sequencer_registry::CallMessage::Register {
                da_address: other_sequencer_da_address,
                amount: SEQUENCE_STAKE,
            },
        ),
        assert: Box::new(move |result, state| {
            // Assert that the sequencer has correctly been registered
            assert!(
                TestSequencerRegistry::default()
                    .is_sender_known(&MockAddress::new(NON_DEFAULT_SEQUENCER_DA_ADDRESS), state)
                    .is_ok_and(|allowed_sequencer| allowed_sequencer.balance_state.is_active()),
                "The sequencer is not registered"
            );
            // Update the other sequencer's balance
            other_sequencer_balance_ref.sub(result.gas_value_used);
            // Assert that the other sequencer balance has been updated
            assert_eq!(
                TestRunner::<RT, S>::bank_gas_balance(&other_sequencer_address, state),
                Some(
                    other_sequencer_balance_ref
                        .get()
                        .checked_sub(SEQUENCE_STAKE)
                        .unwrap()
                )
            );
        }),
    };
    let exit = TransactionTestCase {
        input: additional_sequencer.create_plain_message::<RT, TestSequencerRegistry>(
            sov_sequencer_registry::CallMessage::InitiateWithdrawal {
                da_address: other_sequencer_da_address,
            },
        ),
        assert: Box::new(move |result, state| {
            // Assert that the sequencer has correctly been unregistered
            let Ok(allowed_sequencer) = TestSequencerRegistry::default()
                .is_sender_known(&MockAddress::new(NON_DEFAULT_SEQUENCER_DA_ADDRESS), state)
            else {
                panic!("The sequencer is not registered");
            };
            assert!(
                allowed_sequencer.balance_state.is_pending_withdrawal(),
                "The sequencer {} should be registered and pending withdrawal",
                allowed_sequencer.address
            );
            // Assert that an exit event has been emitted
            assert!(result.events.iter().any(|event| matches!(
                event,
                TestRuntimeEvent::SequencerRegistry(
                    sov_sequencer_registry::Event::InitiatedWithdrawal { sequencer }
                ) if *sequencer == other_sequencer_address
            )));
            other_sequencer_balance_ref_1.sub(result.gas_value_used);
        }),
    };

    let failed_withdrawal = TransactionTestCase {
        input: additional_sequencer.create_plain_message::<RT, TestSequencerRegistry>(
            sov_sequencer_registry::CallMessage::Withdraw {
                da_address: other_sequencer_da_address,
            },
        ),
        assert: Box::new(move |result, _state| {
            let TxEffect::Reverted(contents) = result.tx_receipt else {
                panic!(
                    "Expected transaction to revert, but got: {:?}",
                    result.tx_receipt
                );
            };
            assert!(
                contents
                    .reason
                    .to_string()
                    .contains("Sequencers may not withdraw without first initiating a withdrawal"),
                "Unexpected reason: {}",
                contents.reason
            );
            other_sequencer_balance_ref_2.sub(result.gas_value_used);
        }),
    };

    let failed_withdrawal_2 = TransactionTestCase {
        input: additional_sequencer.create_plain_message::<RT, TestSequencerRegistry>(
            sov_sequencer_registry::CallMessage::Withdraw {
                da_address: other_sequencer_da_address,
            },
        ),
        assert: Box::new(move |result, _state| {
            let TxEffect::Reverted(contents) = result.tx_receipt else {
                panic!(
                    "Expected transaction to revert, but got: {:?}",
                    result.tx_receipt
                );
            };
            assert!(
                contents.reason.to_string().contains(
                    " may not withdraw before the withdrawal is ready. Current visible height"
                ),
                "Unexpected reason: {}",
                contents.reason
            );
            other_sequencer_balance_ref_3.sub(result.gas_value_used);
        }),
    };

    let withdraw = TransactionTestCase {
        input: additional_sequencer.create_plain_message::<RT, TestSequencerRegistry>(
            sov_sequencer_registry::CallMessage::Withdraw {
                da_address: other_sequencer_da_address,
            },
        ),
        assert: Box::new(move |result, state| {
            // Assert that the sequencer has correctly been unregistered
            assert!(
                TestSequencerRegistry::default()
                    .is_sender_known(&MockAddress::new(NON_DEFAULT_SEQUENCER_DA_ADDRESS), state)
                    .is_err(),
                "The sequencer should be unregistered"
            );
            let expected_balance_to_withdraw = SEQUENCE_STAKE;
            // Assert that an exit event has been emitted
            assert!(result.events.iter().any(|event| matches!(
                event,
                TestRuntimeEvent::SequencerRegistry(
                    sov_sequencer_registry::Event::Withdrew { sequencer, amount_withdrawn }
                ) if *sequencer == other_sequencer_address && *amount_withdrawn == expected_balance_to_withdraw.0
            )));
            // Update the other sequencer's balance
            other_sequencer_balance_ref_4.sub(result.gas_value_used);
            // Assert that the other sequencer balance has been updated and that he recovered his bond
            assert_eq!(
                TestRunner::<RT, S>::bank_gas_balance(&other_sequencer_address, state),
                Some(other_sequencer_balance_ref_4.get())
            );
        }),
    };

    runner
        .execute_transaction(register)
        .execute_transaction(failed_withdrawal)
        .execute_transaction(exit)
        .execute_transaction(failed_withdrawal_2)
        .advance_slots(config_value!("DEFERRED_SLOTS_COUNT"))
        .execute_transaction(withdraw);
}

/// Tests that another sequencer can register and exit.
#[test]
fn test_deposit_resets_balance_state() {
    let (roles, mut runner) = setup();

    let additional_sequencer = roles.additional_sequencer;

    let other_sequencer_da_address = MockAddress::new(NON_DEFAULT_SEQUENCER_DA_ADDRESS);

    let register = TransactionTestCase {
        input: additional_sequencer.create_plain_message::<RT, TestSequencerRegistry>(
            sov_sequencer_registry::CallMessage::Register {
                da_address: other_sequencer_da_address,
                amount: SEQUENCE_STAKE,
            },
        ),
        assert: Box::new(move |result, _state| {
            // Assert that the sequencer has correctly been registered
            assert!(result.tx_receipt.is_successful());
        }),
    };
    let exit = TransactionTestCase {
        input: additional_sequencer.create_plain_message::<RT, TestSequencerRegistry>(
            sov_sequencer_registry::CallMessage::InitiateWithdrawal {
                da_address: other_sequencer_da_address,
            },
        ),
        assert: Box::new(move |result, _state| {
            // Assert that the sequencer has correctly been unregistered
            assert!(result.tx_receipt.is_successful());
        }),
    };

    let deposit = TransactionTestCase {
        input: additional_sequencer.create_plain_message::<RT, TestSequencerRegistry>(
            sov_sequencer_registry::CallMessage::Deposit {
                da_address: other_sequencer_da_address,
                amount: Amount::new(1),
            },
        ),
        assert: Box::new(move |result, state| {
            assert!(result.tx_receipt.is_successful());
            assert!(
                TestSequencerRegistry::default()
                    .is_sender_known(&MockAddress::new(NON_DEFAULT_SEQUENCER_DA_ADDRESS), state)
                    .is_ok_and(
                        |allowed_sequencer| allowed_sequencer.balance_state.is_active()
                            && allowed_sequencer.balance
                                == SEQUENCE_STAKE.checked_add(Amount::new(1)).unwrap()
                    ),
                "The sequencer should be unregistered"
            );
        }),
    };

    runner
        .execute_transaction(register)
        .execute_transaction(exit)
        .execute_transaction(deposit);
}

/// Tests that another sequencer cannot exit in their own batch.
#[test]
fn cannot_exit_with_own_batch() {
    let (
        TestRoles {
            default_sequencer, ..
        },
        mut runner,
    ) = setup();

    runner.execute_transaction(TransactionTestCase {
        input: default_sequencer.create_plain_message::<RT, TestSequencerRegistry>(
            CallMessage::InitiateWithdrawal {
                da_address: default_sequencer.da_address,
            },
        ),
        assert: Box::new(move |result, _state| match &result.tx_receipt {
            TxEffect::Reverted(reason) => {
                assert_eq!(
                    reason.reason.to_string(),
                    TestSequencerRegistryError::Custom(
                        CustomError::CannotUnregisterDuringOwnBatch(default_sequencer.da_address,)
                    )
                    .to_string(),
                    "Transaction reverted, but with unexpected reason"
                );
            }
            unexpected => panic!("Expected transaction to revert, but got: {unexpected:?}"),
        }),
    });
}

/// Test that triggers the [`SequencerRegistryError::SuppliedAddressDoesNotMatchTxSender`] error by sending an exit transaction from a different sender.
#[test]
fn test_exit_different_sender_fails() {
    let (
        TestRoles {
            additional_sequencer,
            admin: second_sequencer,
            ..
        },
        mut runner,
    ) = setup();

    let additional_sequencer_register = additional_sequencer
        .create_plain_message::<RT, TestSequencerRegistry>(
            sov_sequencer_registry::CallMessage::Register {
                da_address: NON_DEFAULT_SEQUENCER_DA_ADDRESS.into(),
                amount: SEQUENCE_STAKE,
            },
        );
    let second_sequencer_register = second_sequencer
        .create_plain_message::<RT, TestSequencerRegistry>(
            sov_sequencer_registry::CallMessage::Register {
                da_address: ANOTHER_SEQUENCER_DA_ADDRESS.into(),
                amount: SEQUENCE_STAKE,
            },
        );
    let second_sequencer_address = second_sequencer.address();
    let additional_sequencer_address = additional_sequencer.address();
    runner.execute(additional_sequencer_register);
    runner.execute(second_sequencer_register);

    runner.execute_transaction(TransactionTestCase {
        input: additional_sequencer.create_plain_message::<RT, TestSequencerRegistry>(
            sov_sequencer_registry::CallMessage::InitiateWithdrawal {
                da_address: ANOTHER_SEQUENCER_DA_ADDRESS.into(),
            },
        ),
        assert: Box::new(move |result, _state| match &result.tx_receipt {
            TxEffect::Reverted(reason) => {
                assert_eq!(
                    reason.reason.to_string(),
                    TestSequencerRegistryError::Custom(
                        CustomError::SuppliedAddressDoesNotMatchTxSender {
                            parameter: second_sequencer_address,
                            sender: additional_sequencer_address,
                        },
                    )
                    .to_string(),
                    "Transaction reverted, but with unexpected reason"
                );
            }
            unexpected => panic!("Expected transaction to revert, but got: {unexpected:?}"),
        }),
    });

    runner.execute_transaction(TransactionTestCase {
        input: second_sequencer.create_plain_message::<RT, TestSequencerRegistry>(
            sov_sequencer_registry::CallMessage::InitiateWithdrawal {
                da_address: ANOTHER_SEQUENCER_DA_ADDRESS.into(),
            },
        ),
        assert: Box::new(move |result, _state| {
            assert!(result.tx_receipt.is_successful());
        }),
    });
    runner.advance_slots(config_value!("DEFERRED_SLOTS_COUNT") + 1);
    runner.execute_transaction(TransactionTestCase {
        input: additional_sequencer.create_plain_message::<RT, TestSequencerRegistry>(
            sov_sequencer_registry::CallMessage::Withdraw {
                da_address: ANOTHER_SEQUENCER_DA_ADDRESS.into(),
            },
        ),
        assert: Box::new(move |result, _state| match &result.tx_receipt {
            TxEffect::Reverted(reason) => {
                assert_eq!(
                    reason.reason.to_string(),
                    TestSequencerRegistryError::Custom(
                        CustomError::SuppliedAddressDoesNotMatchTxSender {
                            parameter: second_sequencer.address(),
                            sender: additional_sequencer.address(),
                        },
                    )
                    .to_string(),
                    "Transaction reverted, but with unexpected reason"
                );
            }
            unexpected => panic!("Expected transaction to revert, but got: {unexpected:?}"),
        }),
    });
}

/// By default, the genesis sequencer is also the preferred sequencer. This checks whether the preferred sequencer is returned correctly.
#[test]
fn test_get_preferred_sequencer() {
    let (
        TestRoles {
            default_sequencer, ..
        },
        runner,
    ) = setup();

    runner.query_visible_state(|state| {
        assert_eq!(
            Some(default_sequencer.da_address),
            TestSequencerRegistry::default()
                .get_preferred_sequencer(state)
                .unwrap_infallible()
                .map(|(da, _seq)| da)
        );
    });
}

/// Tests that the preferred sequencer can exit and is removed from the list of preferred sequencers.
#[test]
fn test_get_preferred_sequencer_after_exit() {
    let (
        TestRoles {
            default_sequencer,
            additional_sequencer,
            ..
        },
        mut runner,
    ) = setup();

    let additional_sequencer_da_address = ANOTHER_SEQUENCER_DA_ADDRESS;

    let register_additional_sequencer = additional_sequencer
        .create_plain_message::<RT, TestSequencerRegistry>(
            sov_sequencer_registry::CallMessage::Register {
                da_address: ANOTHER_SEQUENCER_DA_ADDRESS.into(),
                amount: SEQUENCE_STAKE,
            },
        );

    let initiate_withdrawal_default_sequencer = default_sequencer
        .create_plain_message::<RT, TestSequencerRegistry>(
            sov_sequencer_registry::CallMessage::InitiateWithdrawal {
                da_address: default_sequencer.da_address,
            },
        );

    let exit_default_sequencer = default_sequencer
        .create_plain_message::<RT, TestSequencerRegistry>(
            sov_sequencer_registry::CallMessage::Withdraw {
                da_address: default_sequencer.da_address,
            },
        );

    // Register the additional sequencer
    runner.execute(register_additional_sequencer);

    runner.config.sequencer_da_address = additional_sequencer_da_address.into();
    // Then exit the normal sequencer
    runner.execute(initiate_withdrawal_default_sequencer);
    runner.advance_slots(config_value!("DEFERRED_SLOTS_COUNT"));
    runner.execute(exit_default_sequencer);

    // Check that the normal sequencer is no longer the preferred sequencer
    runner.query_visible_state(|state| {
        assert_eq!(
            None,
            TestSequencerRegistry::default()
                .get_preferred_sequencer(state)
                .unwrap_infallible()
        );
    });
}

/// Tests that increasing the stake amount fails if sequencer does not have enough funds.
#[test]
fn test_balance_increase_fails_if_insufficient_funds() {
    let (
        TestRoles {
            default_sequencer, ..
        },
        mut runner,
    ) = setup();

    let default_sequencer_balance = default_sequencer.user_info.available_gas_balance;
    let default_sequencer_address = default_sequencer.user_info.address();

    runner.execute_transaction(TransactionTestCase {
        input: default_sequencer.create_plain_message::<RT, TestSequencerRegistry>(
            sov_sequencer_registry::CallMessage::Deposit {
                da_address: default_sequencer.da_address,
                amount: default_sequencer_balance
                    .checked_add(SEQUENCE_STAKE)
                    .unwrap(),
            },
        ),
        assert: Box::new(move |result, _state| match &result.tx_receipt {
            TxEffect::Reverted(reason) => {
                assert_eq!(
                    reason.reason.to_string(),
                    TestSequencerRegistryError::InsufficientFundsToTopUpAccount {
                        address: default_sequencer_address,
                        amount_to_add: default_sequencer_balance
                            .checked_add(SEQUENCE_STAKE)
                            .unwrap(),
                    }
                    .to_string(),
                    "Transaction reverted, but with unexpected reason"
                );
            }
            unexpected => panic!("Expected transaction to revert, but got: {unexpected:?}"),
        }),
    });
}

#[test]
fn test_non_registered_sequencer_is_not_allowed() {
    let (_, runner) = setup();

    runner.query_visible_state(|state| {
        assert!(
            TestSequencerRegistry::default()
                .is_sender_known(&MockAddress::from(NON_DEFAULT_SEQUENCER_DA_ADDRESS), state)
                .is_err(),
            "Non-registered sequencers should not be allowed"
        );
    });
}

#[test]
fn test_non_registered_sequencer_cannot_send_batches() {
    let (TestRoles { admin, .. }, mut runner) = setup();

    runner.config.sequencer_da_address = NON_DEFAULT_SEQUENCER_DA_ADDRESS.into();

    let (outcome, _) = runner.execute(BatchType(vec![admin
        .create_plain_message::<RT, TestSequencerRegistry>(
            sov_sequencer_registry::CallMessage::Register {
                da_address: NON_DEFAULT_SEQUENCER_DA_ADDRESS.into(),
                amount: SEQUENCE_STAKE,
            },
        )]));

    assert!(outcome.batch_receipts.is_empty());
}

/// We should not be able to increase the stake amount (through deposit) for a non-registered sequencer.
#[test]
fn test_balance_increase_fails_for_unknown_sequencer() {
    let (
        TestRoles {
            additional_sequencer,
            ..
        },
        mut runner,
    ) = setup();

    runner.execute_transaction(TransactionTestCase {
        input: additional_sequencer.create_plain_message::<RT, TestSequencerRegistry>(
            sov_sequencer_registry::CallMessage::Deposit {
                da_address: NON_DEFAULT_SEQUENCER_DA_ADDRESS.into(),
                amount: SEQUENCE_STAKE,
            },
        ),
        assert: Box::new(move |result, _state| match &result.tx_receipt {
            TxEffect::Reverted(reason) => {
                assert_eq!(
                    reason.reason.to_string(),
                    TestSequencerRegistryError::IsNotRegistered(MockAddress::from(
                        NON_DEFAULT_SEQUENCER_DA_ADDRESS,
                    ))
                    .to_string(),
                    "Transaction reverted, but with unexpected reason"
                );
            }
            unexpected => panic!("Expected transaction to revert, but got: {unexpected:?}"),
        }),
    });
}
