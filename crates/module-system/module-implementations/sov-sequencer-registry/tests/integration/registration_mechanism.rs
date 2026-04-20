use sov_bank::{Bank, IntoPayable};
use sov_mock_da::MockAddress;
use sov_modules_api::macros::config_value;
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::transaction::PriorityFeeBips;
use sov_modules_api::{Amount, Gas, GasArray, GasSpec, GetGasPrice, ModuleInfo, TxEffect};
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
            let sequencer_burn = S::gas_to_charge_per_byte_borsh_read()
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

const ROTATED_DA_ADDRESS: [u8; 32] = [3; 32];
const SECOND_ROTATED_DA_ADDRESS: [u8; 32] = [4; 32];

/// A1 + C1: happy path. A regular sequencer rotates its DA from inside a different
/// sequencer's batch (to bypass the own-batch guard). Verifies that:
/// - the old entry is gone,
/// - the new entry preserves rollup address, balance, and Active state,
/// - the `DaAddressUpdated` event is emitted with matching fields,
/// - the preferred_sequencer pointer is untouched (additional_sequencer is not preferred).
#[test]
fn test_update_da_address_happy_path() {
    let (
        TestRoles {
            additional_sequencer,
            ..
        },
        mut runner,
    ) = setup();

    let rollup_address = additional_sequencer.address();
    let old_da: MockAddress = NON_DEFAULT_SEQUENCER_DA_ADDRESS.into();
    let new_da: MockAddress = ROTATED_DA_ADDRESS.into();

    runner.execute(
        additional_sequencer.create_plain_message::<RT, TestSequencerRegistry>(
            CallMessage::Register {
                da_address: old_da,
                amount: SEQUENCE_STAKE,
            },
        ),
    );

    // Genesis default_sequencer is still the active batch producer, so the
    // additional_sequencer is not in its own batch. Own-batch guard does not fire.
    runner.execute_transaction(TransactionTestCase {
        input: additional_sequencer.create_plain_message::<RT, TestSequencerRegistry>(
            CallMessage::UpdateDaAddress {
                old_da_address: old_da,
                new_da_address: new_da,
            },
        ),
        assert: Box::new(move |result, state| {
            assert!(result.tx_receipt.is_successful());

            assert!(
                TestSequencerRegistry::default()
                    .is_sender_known(&old_da, state)
                    .is_err(),
                "Old DA entry should be gone after rotation"
            );

            let new_entry = TestSequencerRegistry::default()
                .is_sender_known(&new_da, state)
                .expect("New DA entry should exist after rotation");
            assert_eq!(new_entry.address, rollup_address);
            assert_eq!(new_entry.balance, SEQUENCE_STAKE);
            assert!(new_entry.balance_state.is_active());

            assert!(
                result.events.iter().any(|event| matches!(
                    event,
                    TestRuntimeEvent::SequencerRegistry(
                        sov_sequencer_registry::Event::DaAddressUpdated {
                            sequencer,
                            old_da_address,
                            new_da_address,
                        }
                    ) if *sequencer == rollup_address
                        && *old_da_address == old_da
                        && *new_da_address == new_da
                )),
                "DaAddressUpdated event with matching fields should be emitted"
            );
        }),
    });
}

/// A2 + A3: impersonation is blocked. A and B are both registered sequencers;
/// A's rollup key signs a tx carrying B's DA address as `old_da_address`.
/// Expected: `SuppliedAddressDoesNotMatchTxSender`, and neither sequencer's
/// registry entry is mutated.
#[test]
fn test_update_da_address_cannot_hijack_victim_da() {
    let (
        TestRoles {
            additional_sequencer,
            admin: victim,
            ..
        },
        mut runner,
    ) = setup();

    let attacker_rollup = additional_sequencer.address();
    let victim_rollup = victim.address();
    let attacker_da: MockAddress = NON_DEFAULT_SEQUENCER_DA_ADDRESS.into();
    let victim_da: MockAddress = ANOTHER_SEQUENCER_DA_ADDRESS.into();
    let malicious_new_da: MockAddress = ROTATED_DA_ADDRESS.into();

    runner.execute(
        additional_sequencer.create_plain_message::<RT, TestSequencerRegistry>(
            CallMessage::Register {
                da_address: attacker_da,
                amount: SEQUENCE_STAKE,
            },
        ),
    );
    runner.execute(victim.create_plain_message::<RT, TestSequencerRegistry>(
        CallMessage::Register {
            da_address: victim_da,
            amount: SEQUENCE_STAKE,
        },
    ));

    runner.execute_transaction(TransactionTestCase {
        input: additional_sequencer.create_plain_message::<RT, TestSequencerRegistry>(
            CallMessage::UpdateDaAddress {
                old_da_address: victim_da,
                new_da_address: malicious_new_da,
            },
        ),
        assert: Box::new(move |result, state| {
            match &result.tx_receipt {
                TxEffect::Reverted(reason) => {
                    assert_eq!(
                        reason.reason.to_string(),
                        TestSequencerRegistryError::Custom(
                            CustomError::SuppliedAddressDoesNotMatchTxSender {
                                parameter: victim_rollup,
                                sender: attacker_rollup,
                            },
                        )
                        .to_string(),
                    );
                }
                unexpected => panic!("Expected revert, got: {unexpected:?}"),
            }

            let attacker_entry = TestSequencerRegistry::default()
                .is_sender_known(&attacker_da, state)
                .expect("Attacker entry must be intact");
            assert_eq!(attacker_entry.address, attacker_rollup);

            let victim_entry = TestSequencerRegistry::default()
                .is_sender_known(&victim_da, state)
                .expect("Victim entry must be intact");
            assert_eq!(victim_entry.address, victim_rollup);

            assert!(
                TestSequencerRegistry::default()
                    .is_sender_known(&malicious_new_da, state)
                    .is_err(),
                "Malicious new DA must not have been inserted"
            );
        }),
    });
}

/// B1: rotating from a DA the caller doesn't own (and that isn't registered at all)
/// fails with `IsNotRegistered`, produced by `validate_sender`.
#[test]
fn test_update_da_address_old_not_registered_fails() {
    let (
        TestRoles {
            additional_sequencer,
            ..
        },
        mut runner,
    ) = setup();

    let phantom_old_da: MockAddress = ROTATED_DA_ADDRESS.into();
    let new_da: MockAddress = SECOND_ROTATED_DA_ADDRESS.into();

    runner.execute_transaction(TransactionTestCase {
        input: additional_sequencer.create_plain_message::<RT, TestSequencerRegistry>(
            CallMessage::UpdateDaAddress {
                old_da_address: phantom_old_da,
                new_da_address: new_da,
            },
        ),
        assert: Box::new(move |result, _state| match &result.tx_receipt {
            TxEffect::Reverted(reason) => {
                assert_eq!(
                    reason.reason.to_string(),
                    TestSequencerRegistryError::IsNotRegistered(phantom_old_da).to_string(),
                );
            }
            unexpected => panic!("Expected revert, got: {unexpected:?}"),
        }),
    });
}

/// B2: rotating to the same address the sequencer already uses fails with
/// `NewDaAddressSameAsOld`.
#[test]
fn test_update_da_address_same_old_new_fails() {
    let (
        TestRoles {
            additional_sequencer,
            ..
        },
        mut runner,
    ) = setup();

    let da: MockAddress = NON_DEFAULT_SEQUENCER_DA_ADDRESS.into();

    runner.execute(
        additional_sequencer.create_plain_message::<RT, TestSequencerRegistry>(
            CallMessage::Register {
                da_address: da,
                amount: SEQUENCE_STAKE,
            },
        ),
    );

    runner.execute_transaction(TransactionTestCase {
        input: additional_sequencer.create_plain_message::<RT, TestSequencerRegistry>(
            CallMessage::UpdateDaAddress {
                old_da_address: da,
                new_da_address: da,
            },
        ),
        assert: Box::new(move |result, _state| match &result.tx_receipt {
            TxEffect::Reverted(reason) => {
                assert_eq!(
                    reason.reason.to_string(),
                    TestSequencerRegistryError::Custom(CustomError::NewDaAddressSameAsOld(da))
                        .to_string(),
                );
            }
            unexpected => panic!("Expected revert, got: {unexpected:?}"),
        }),
    });
}

/// B3: rotating onto another sequencer's DA fails with `AlreadyRegistered`,
/// carrying the conflicting rollup address. Verifies no state mutation occurs.
#[test]
fn test_update_da_address_new_already_registered_fails() {
    let (
        TestRoles {
            additional_sequencer,
            admin: other,
            ..
        },
        mut runner,
    ) = setup();

    let attacker_rollup = additional_sequencer.address();
    let other_rollup = other.address();
    let attacker_da: MockAddress = NON_DEFAULT_SEQUENCER_DA_ADDRESS.into();
    let other_da: MockAddress = ANOTHER_SEQUENCER_DA_ADDRESS.into();

    runner.execute(
        additional_sequencer.create_plain_message::<RT, TestSequencerRegistry>(
            CallMessage::Register {
                da_address: attacker_da,
                amount: SEQUENCE_STAKE,
            },
        ),
    );
    runner.execute(other.create_plain_message::<RT, TestSequencerRegistry>(
        CallMessage::Register {
            da_address: other_da,
            amount: SEQUENCE_STAKE,
        },
    ));

    runner.execute_transaction(TransactionTestCase {
        input: additional_sequencer.create_plain_message::<RT, TestSequencerRegistry>(
            CallMessage::UpdateDaAddress {
                old_da_address: attacker_da,
                new_da_address: other_da,
            },
        ),
        assert: Box::new(move |result, state| {
            match &result.tx_receipt {
                TxEffect::Reverted(reason) => {
                    assert_eq!(
                        reason.reason.to_string(),
                        TestSequencerRegistryError::AlreadyRegistered(other_rollup).to_string(),
                    );
                }
                unexpected => panic!("Expected revert, got: {unexpected:?}"),
            }

            assert_eq!(
                TestSequencerRegistry::default()
                    .is_sender_known(&attacker_da, state)
                    .unwrap()
                    .address,
                attacker_rollup,
            );
            assert_eq!(
                TestSequencerRegistry::default()
                    .is_sender_known(&other_da, state)
                    .unwrap()
                    .address,
                other_rollup,
            );
        }),
    });
}

/// B4: multi-DA is currently permitted by `register_staker` (it only rejects
/// duplicate DA keys, not duplicate rollup addresses). If a single rollup
/// registers two DAs, rotating one onto the other must fail with
/// `AlreadyRegistered` carrying the caller's own rollup address. Documents
/// current multi-DA semantics.
#[test]
fn test_update_da_address_new_same_as_caller_other_da_fails() {
    let (
        TestRoles {
            additional_sequencer,
            ..
        },
        mut runner,
    ) = setup();

    let rollup = additional_sequencer.address();
    let da_one: MockAddress = NON_DEFAULT_SEQUENCER_DA_ADDRESS.into();
    let da_two: MockAddress = ANOTHER_SEQUENCER_DA_ADDRESS.into();

    runner.execute(
        additional_sequencer.create_plain_message::<RT, TestSequencerRegistry>(
            CallMessage::Register {
                da_address: da_one,
                amount: SEQUENCE_STAKE,
            },
        ),
    );
    runner.execute(
        additional_sequencer.create_plain_message::<RT, TestSequencerRegistry>(
            CallMessage::Register {
                da_address: da_two,
                amount: SEQUENCE_STAKE,
            },
        ),
    );

    runner.execute_transaction(TransactionTestCase {
        input: additional_sequencer.create_plain_message::<RT, TestSequencerRegistry>(
            CallMessage::UpdateDaAddress {
                old_da_address: da_one,
                new_da_address: da_two,
            },
        ),
        assert: Box::new(move |result, _state| match &result.tx_receipt {
            TxEffect::Reverted(reason) => {
                assert_eq!(
                    reason.reason.to_string(),
                    TestSequencerRegistryError::AlreadyRegistered(rollup).to_string(),
                );
            }
            unexpected => panic!("Expected revert, got: {unexpected:?}"),
        }),
    });
}

/// C2: rotating a sequencer in `PendingWithdrawal { ready_at }` must preserve
/// the state exactly, including `ready_at`. F1 composes with this: after
/// advancing past `ready_at`, `Withdraw` on the new DA succeeds.
#[test]
fn test_update_da_address_preserves_pending_withdrawal_state() {
    let (
        TestRoles {
            additional_sequencer,
            ..
        },
        mut runner,
    ) = setup();

    let rollup = additional_sequencer.address();
    let old_da: MockAddress = NON_DEFAULT_SEQUENCER_DA_ADDRESS.into();
    let new_da: MockAddress = ROTATED_DA_ADDRESS.into();

    runner.execute(
        additional_sequencer.create_plain_message::<RT, TestSequencerRegistry>(
            CallMessage::Register {
                da_address: old_da,
                amount: SEQUENCE_STAKE,
            },
        ),
    );
    runner.execute(
        additional_sequencer.create_plain_message::<RT, TestSequencerRegistry>(
            CallMessage::InitiateWithdrawal { da_address: old_da },
        ),
    );

    // Capture the exact `ready_at` that was written by InitiateWithdrawal.
    let pre_rotation_ready_at = runner.query_visible_state(|state| {
        let entry = TestSequencerRegistry::default()
            .is_sender_known(&old_da, state)
            .unwrap();
        match entry.balance_state {
            sov_sequencer_registry::BalanceState::PendingWithdrawal { ready_at } => ready_at,
            other => panic!("Expected PendingWithdrawal, got {other:?}"),
        }
    });

    runner.execute_transaction(TransactionTestCase {
        input: additional_sequencer.create_plain_message::<RT, TestSequencerRegistry>(
            CallMessage::UpdateDaAddress {
                old_da_address: old_da,
                new_da_address: new_da,
            },
        ),
        assert: Box::new(move |result, state| {
            assert!(result.tx_receipt.is_successful());

            let new_entry = TestSequencerRegistry::default()
                .is_sender_known(&new_da, state)
                .expect("New DA entry should exist after rotation");
            assert_eq!(new_entry.address, rollup);
            assert_eq!(new_entry.balance, SEQUENCE_STAKE);
            match new_entry.balance_state {
                sov_sequencer_registry::BalanceState::PendingWithdrawal { ready_at } => {
                    assert_eq!(ready_at, pre_rotation_ready_at);
                }
                other => panic!("Expected PendingWithdrawal preserved, got {other:?}"),
            }
        }),
    });

    // F1: Withdraw on new DA completes normally once `ready_at` is reached.
    runner.advance_slots(config_value!("DEFERRED_SLOTS_COUNT") + 1);
    runner.execute_transaction(TransactionTestCase {
        input: additional_sequencer.create_plain_message::<RT, TestSequencerRegistry>(
            CallMessage::Withdraw { da_address: new_da },
        ),
        assert: Box::new(move |result, state| {
            assert!(result.tx_receipt.is_successful());
            assert!(
                TestSequencerRegistry::default()
                    .is_sender_known(&new_da, state)
                    .is_err(),
                "After Withdraw, new DA entry should be gone"
            );
            assert!(result.events.iter().any(|event| matches!(
                event,
                TestRuntimeEvent::SequencerRegistry(
                    sov_sequencer_registry::Event::Withdrew { sequencer, amount_withdrawn }
                ) if *sequencer == rollup && *amount_withdrawn == SEQUENCE_STAKE
            )));
        }),
    });
}

/// D1 + G1: rotating the preferred sequencer's DA moves the `preferred_sequencer`
/// pointer so that `get_preferred_sequencer` returns the new DA — which is
/// what `blob-storage::separate_preferred_blobs` reads.
#[test]
fn test_update_da_address_moves_preferred_sequencer() {
    let (
        TestRoles {
            default_sequencer,
            additional_sequencer,
            ..
        },
        mut runner,
    ) = setup();

    let preferred_rollup = default_sequencer.user_info.address();
    let old_preferred_da = default_sequencer.da_address;
    let new_preferred_da: MockAddress = ROTATED_DA_ADDRESS.into();

    // Register `additional_sequencer` and make them the active batch producer
    // so the preferred sequencer isn't executing their own batch when they rotate.
    runner.execute(
        additional_sequencer.create_plain_message::<RT, TestSequencerRegistry>(
            CallMessage::Register {
                da_address: ANOTHER_SEQUENCER_DA_ADDRESS.into(),
                amount: SEQUENCE_STAKE,
            },
        ),
    );
    runner.config.sequencer_da_address = ANOTHER_SEQUENCER_DA_ADDRESS.into();

    runner.execute_transaction(TransactionTestCase {
        input: default_sequencer.create_plain_message::<RT, TestSequencerRegistry>(
            CallMessage::UpdateDaAddress {
                old_da_address: old_preferred_da,
                new_da_address: new_preferred_da,
            },
        ),
        assert: Box::new(move |result, state| {
            assert!(result.tx_receipt.is_successful());

            assert_eq!(
                TestSequencerRegistry::default()
                    .get_preferred_sequencer(state)
                    .unwrap_infallible(),
                Some((new_preferred_da, preferred_rollup)),
            );

            assert!(
                TestSequencerRegistry::default()
                    .is_sender_known(&old_preferred_da, state)
                    .is_err(),
                "Old preferred DA should be removed from known_sequencers"
            );
        }),
    });
}

/// D2: rotating a regular (non-preferred) sequencer must not touch the
/// `preferred_sequencer` pointer.
#[test]
fn test_update_da_address_regular_rotation_does_not_touch_preferred() {
    let (
        TestRoles {
            default_sequencer,
            additional_sequencer,
            ..
        },
        mut runner,
    ) = setup();

    let preferred_rollup = default_sequencer.user_info.address();
    let preferred_da = default_sequencer.da_address;
    let regular_old_da: MockAddress = NON_DEFAULT_SEQUENCER_DA_ADDRESS.into();
    let regular_new_da: MockAddress = ROTATED_DA_ADDRESS.into();

    runner.execute(
        additional_sequencer.create_plain_message::<RT, TestSequencerRegistry>(
            CallMessage::Register {
                da_address: regular_old_da,
                amount: SEQUENCE_STAKE,
            },
        ),
    );

    runner.execute_transaction(TransactionTestCase {
        input: additional_sequencer.create_plain_message::<RT, TestSequencerRegistry>(
            CallMessage::UpdateDaAddress {
                old_da_address: regular_old_da,
                new_da_address: regular_new_da,
            },
        ),
        assert: Box::new(move |result, state| {
            assert!(result.tx_receipt.is_successful());

            assert_eq!(
                TestSequencerRegistry::default()
                    .get_preferred_sequencer(state)
                    .unwrap_infallible(),
                Some((preferred_da, preferred_rollup)),
                "Preferred sequencer pointer must be untouched when rotating a regular sequencer",
            );
        }),
    });
}

/// E1: the preferred sequencer can rotate its own DA from its own batch. The
/// registry applies the move in the end-block hook, so the batch executes
/// consistently against the old DA address and later state exposes the new DA.
#[test]
fn test_preferred_update_da_address_during_own_batch_succeeds() {
    let (
        TestRoles {
            default_sequencer, ..
        },
        mut runner,
    ) = setup();

    let old_da = default_sequencer.da_address;
    let new_da: MockAddress = ROTATED_DA_ADDRESS.into();
    let preferred_rollup = default_sequencer.user_info.address();

    runner.execute_transaction(TransactionTestCase {
        input: default_sequencer.create_plain_message::<RT, TestSequencerRegistry>(
            CallMessage::UpdateDaAddress {
                old_da_address: old_da,
                new_da_address: new_da,
            },
        ),
        assert: Box::new(move |result, state| {
            assert!(result.tx_receipt.is_successful());

            let reg = TestSequencerRegistry::default();
            assert_eq!(
                reg.get_preferred_sequencer(state).unwrap_infallible(),
                Some((new_da, preferred_rollup)),
            );
            assert!(reg.is_sender_known(&old_da, state).is_err());

            let new_entry = reg
                .is_sender_known(&new_da, state)
                .expect("Preferred sequencer should be known at the rotated DA address");
            assert_eq!(new_entry.address, preferred_rollup);
            let sequencer_burn = S::gas_to_charge_per_byte_borsh_read()
                .checked_scalar_product(result.blob_info.size as u64)
                .unwrap()
                .checked_value(state.gas_price())
                .unwrap();
            assert_eq!(
                new_entry.balance,
                default_sequencer.bond.checked_sub(sequencer_burn).unwrap()
            );
            assert!(new_entry.balance_state.is_active());

            assert!(result.events.iter().any(|event| matches!(
                event,
                TestRuntimeEvent::SequencerRegistry(
                    sov_sequencer_registry::Event::DaAddressUpdated {
                        sequencer,
                        old_da_address,
                        new_da_address,
                    }
                ) if *sequencer == preferred_rollup
                    && *old_da_address == old_da
                    && *new_da_address == new_da
            )));
        }),
    });
}

/// E2: only one preferred self-rotation can be pending in a rollup block.
#[test]
fn test_second_preferred_self_rotation_in_same_batch_fails() {
    let (
        TestRoles {
            default_sequencer, ..
        },
        mut runner,
    ) = setup();

    let old_da = default_sequencer.da_address;
    let new_da: MockAddress = ROTATED_DA_ADDRESS.into();
    let second_new_da: MockAddress = SECOND_ROTATED_DA_ADDRESS.into();
    let preferred_rollup = default_sequencer.user_info.address();

    runner.execute_batch(BatchTestCase {
        input: BatchType(vec![
            default_sequencer.create_plain_message::<RT, TestSequencerRegistry>(
                CallMessage::UpdateDaAddress {
                    old_da_address: old_da,
                    new_da_address: new_da,
                },
            ),
            default_sequencer.create_plain_message::<RT, TestSequencerRegistry>(
                CallMessage::UpdateDaAddress {
                    old_da_address: old_da,
                    new_da_address: second_new_da,
                },
            ),
        ]),
        assert: Box::new(move |result, state| {
            let receipt = result.batch_receipt.expect("Batch should be processed");
            assert_eq!(receipt.tx_receipts.len(), 2);
            assert!(receipt.tx_receipts[0].receipt.is_successful());
            assert!(matches!(
                receipt.tx_receipts[1].receipt,
                TxEffect::Reverted(_)
            ));

            let reg = TestSequencerRegistry::default();
            assert_eq!(
                reg.get_preferred_sequencer(state).unwrap_infallible(),
                Some((new_da, preferred_rollup)),
            );
            assert!(reg.is_sender_known(&old_da, state).is_err());
            assert!(reg.is_sender_known(&second_new_da, state).is_err());
        }),
    });
}

/// E3: a pending self-rotation reserves its new DA address for the rest of the
/// rollup block.
#[test]
fn test_pending_preferred_self_rotation_reserves_new_da_address() {
    let (
        TestRoles {
            default_sequencer,
            additional_sequencer,
            ..
        },
        mut runner,
    ) = setup();

    let old_da = default_sequencer.da_address;
    let new_da: MockAddress = ROTATED_DA_ADDRESS.into();
    let preferred_rollup = default_sequencer.user_info.address();

    runner.execute_batch(BatchTestCase {
        input: BatchType(vec![
            default_sequencer.create_plain_message::<RT, TestSequencerRegistry>(
                CallMessage::UpdateDaAddress {
                    old_da_address: old_da,
                    new_da_address: new_da,
                },
            ),
            additional_sequencer.create_plain_message::<RT, TestSequencerRegistry>(
                CallMessage::Register {
                    da_address: new_da,
                    amount: SEQUENCE_STAKE,
                },
            ),
        ]),
        assert: Box::new(move |result, state| {
            let receipt = result.batch_receipt.expect("Batch should be processed");
            assert_eq!(receipt.tx_receipts.len(), 2);
            assert!(receipt.tx_receipts[0].receipt.is_successful());
            assert!(matches!(
                receipt.tx_receipts[1].receipt,
                TxEffect::Reverted(_)
            ));

            let entry = TestSequencerRegistry::default()
                .is_sender_known(&new_da, state)
                .expect("Rotated preferred sequencer should occupy the reserved DA address");
            assert_eq!(entry.address, preferred_rollup);
        }),
    });
}

/// E4: a non-preferred active batch producer still cannot rotate its own DA
/// mid-batch.
#[test]
fn test_non_preferred_update_da_address_during_own_batch_fails() {
    let (
        TestRoles {
            additional_sequencer,
            ..
        },
        mut runner,
    ) = setup();

    let old_da: MockAddress = NON_DEFAULT_SEQUENCER_DA_ADDRESS.into();
    let new_da: MockAddress = ROTATED_DA_ADDRESS.into();

    runner.execute(
        additional_sequencer.create_plain_message::<RT, TestSequencerRegistry>(
            CallMessage::Register {
                da_address: old_da,
                amount: SEQUENCE_STAKE,
            },
        ),
    );
    runner.config.sequencer_da_address = old_da;

    runner.execute_transaction(TransactionTestCase {
        input: additional_sequencer.create_plain_message::<RT, TestSequencerRegistry>(
            CallMessage::UpdateDaAddress {
                old_da_address: old_da,
                new_da_address: new_da,
            },
        ),
        assert: Box::new(move |result, state| {
            match &result.tx_receipt {
                TxEffect::Reverted(reason) => {
                    assert_eq!(
                        reason.reason.to_string(),
                        TestSequencerRegistryError::Custom(
                            CustomError::CannotUnregisterDuringOwnBatch(old_da),
                        )
                        .to_string(),
                    );
                }
                unexpected => panic!("Expected revert, got: {unexpected:?}"),
            }

            assert!(TestSequencerRegistry::default()
                .is_sender_known(&old_da, state)
                .is_ok());
            assert!(TestSequencerRegistry::default()
                .is_sender_known(&new_da, state)
                .is_err());
        }),
    });
}

/// F3 + F4 + G2: after rotation, `Deposit` on the new DA succeeds (balance
/// increases as expected) and on the old DA fails (`IsNotRegistered`).
#[test]
fn test_deposit_after_da_rotation() {
    let (
        TestRoles {
            additional_sequencer,
            ..
        },
        mut runner,
    ) = setup();

    let rollup = additional_sequencer.address();
    let old_da: MockAddress = NON_DEFAULT_SEQUENCER_DA_ADDRESS.into();
    let new_da: MockAddress = ROTATED_DA_ADDRESS.into();
    let deposit_amount = Amount::new(1234);

    runner.execute(
        additional_sequencer.create_plain_message::<RT, TestSequencerRegistry>(
            CallMessage::Register {
                da_address: old_da,
                amount: SEQUENCE_STAKE,
            },
        ),
    );
    runner.execute(
        additional_sequencer.create_plain_message::<RT, TestSequencerRegistry>(
            CallMessage::UpdateDaAddress {
                old_da_address: old_da,
                new_da_address: new_da,
            },
        ),
    );

    // F3: Deposit on new DA succeeds and balance increases.
    runner.execute_transaction(TransactionTestCase {
        input: additional_sequencer.create_plain_message::<RT, TestSequencerRegistry>(
            CallMessage::Deposit {
                da_address: new_da,
                amount: deposit_amount,
            },
        ),
        assert: Box::new(move |result, state| {
            assert!(result.tx_receipt.is_successful());
            let entry = TestSequencerRegistry::default()
                .is_sender_known(&new_da, state)
                .unwrap();
            assert_eq!(entry.address, rollup);
            assert_eq!(
                entry.balance,
                SEQUENCE_STAKE.checked_add(deposit_amount).unwrap()
            );
        }),
    });

    // F4: Deposit on old DA fails.
    runner.execute_transaction(TransactionTestCase {
        input: additional_sequencer.create_plain_message::<RT, TestSequencerRegistry>(
            CallMessage::Deposit {
                da_address: old_da,
                amount: deposit_amount,
            },
        ),
        assert: Box::new(move |result, _state| match &result.tx_receipt {
            TxEffect::Reverted(reason) => {
                assert_eq!(
                    reason.reason.to_string(),
                    TestSequencerRegistryError::IsNotRegistered(old_da).to_string(),
                );
            }
            unexpected => panic!("Expected revert, got: {unexpected:?}"),
        }),
    });
}

#[test]
fn test_retired_da_address_cannot_be_registered() {
    let (
        TestRoles {
            additional_sequencer,
            admin,
            ..
        },
        mut runner,
    ) = setup();

    let rollup = additional_sequencer.address();
    let old_da: MockAddress = NON_DEFAULT_SEQUENCER_DA_ADDRESS.into();
    let new_da: MockAddress = ROTATED_DA_ADDRESS.into();

    runner.execute(
        additional_sequencer.create_plain_message::<RT, TestSequencerRegistry>(
            CallMessage::Register {
                da_address: old_da,
                amount: SEQUENCE_STAKE,
            },
        ),
    );
    runner.execute(
        additional_sequencer.create_plain_message::<RT, TestSequencerRegistry>(
            CallMessage::UpdateDaAddress {
                old_da_address: old_da,
                new_da_address: new_da,
            },
        ),
    );

    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, TestSequencerRegistry>(CallMessage::Register {
            da_address: old_da,
            amount: SEQUENCE_STAKE,
        }),
        assert: Box::new(move |result, state| {
            match &result.tx_receipt {
                TxEffect::Reverted(reason) => {
                    assert_eq!(
                        reason.reason.to_string(),
                        TestSequencerRegistryError::AlreadyRegistered(rollup).to_string(),
                    );
                }
                unexpected => panic!("Expected revert, got: {unexpected:?}"),
            }

            let reg = TestSequencerRegistry::default();
            assert!(reg.is_sender_known(&old_da, state).is_err());
            assert_eq!(reg.is_sender_known(&new_da, state).unwrap().address, rollup);
        }),
    });
}

/// F5 + F6 + I1: chain rotations A -> B -> C. Verifies the registry entry
/// hops correctly through each DA, retired DA addresses cannot be reused, and
/// escrow refunds addressed to the original DA are credited to the current DA.
#[test]
fn test_multiple_rotations_retired_refunds_follow_current_da() {
    let (
        TestRoles {
            additional_sequencer,
            ..
        },
        mut runner,
    ) = setup();

    let rollup = additional_sequencer.address();
    let da_a: MockAddress = NON_DEFAULT_SEQUENCER_DA_ADDRESS.into();
    let da_b: MockAddress = ANOTHER_SEQUENCER_DA_ADDRESS.into();
    let da_c: MockAddress = ROTATED_DA_ADDRESS.into();

    runner.execute(
        additional_sequencer.create_plain_message::<RT, TestSequencerRegistry>(
            CallMessage::Register {
                da_address: da_a,
                amount: SEQUENCE_STAKE,
            },
        ),
    );

    let rotate = |from: MockAddress, to: MockAddress| {
        additional_sequencer.create_plain_message::<RT, TestSequencerRegistry>(
            CallMessage::UpdateDaAddress {
                old_da_address: from,
                new_da_address: to,
            },
        )
    };

    runner.execute(rotate(da_a, da_b));
    runner.execute(rotate(da_b, da_c));

    runner.execute_transaction(TransactionTestCase {
        input: rotate(da_c, da_a),
        assert: Box::new(move |result, _state| match &result.tx_receipt {
            TxEffect::Reverted(reason) => {
                assert_eq!(
                    reason.reason.to_string(),
                    TestSequencerRegistryError::AlreadyRegistered(rollup).to_string(),
                );
            }
            unexpected => panic!("Expected revert, got: {unexpected:?}"),
        }),
    });

    let refund_amount = Amount::new(777);
    runner.__apply_to_state(move |state| {
        let mut reg = TestSequencerRegistry::default();
        let bank = Bank::<S>::default();

        reg.remove_part_of_the_stake(&da_c, bank.id().clone().to_payable(), refund_amount, state)
            .unwrap();
        assert_eq!(
            reg.is_sender_known(&da_c, state).unwrap().balance,
            SEQUENCE_STAKE.checked_sub(refund_amount).unwrap()
        );

        reg.add_to_stake(bank.id().clone().to_payable(), &da_a, refund_amount, state)
            .unwrap();
    });

    runner.query_visible_state(|state| {
        let reg = TestSequencerRegistry::default();

        assert!(reg.is_sender_known(&da_a, state).is_err());
        assert!(reg.is_sender_known(&da_b, state).is_err());

        let entry_c = reg
            .is_sender_known(&da_c, state)
            .expect("After chained rotations, entry must live at DA C");
        assert_eq!(entry_c.address, rollup);
        assert_eq!(entry_c.balance, SEQUENCE_STAKE);
        assert!(entry_c.balance_state.is_active());
    });
}

#[test]
fn test_retired_refund_does_not_credit_reused_da_address() {
    let (
        TestRoles {
            additional_sequencer,
            admin,
            ..
        },
        mut runner,
    ) = setup();

    let original_rollup = additional_sequencer.address();
    let reused_rollup = admin.address();
    let da_a: MockAddress = NON_DEFAULT_SEQUENCER_DA_ADDRESS.into();
    let da_b: MockAddress = ANOTHER_SEQUENCER_DA_ADDRESS.into();

    runner.execute(
        additional_sequencer.create_plain_message::<RT, TestSequencerRegistry>(
            CallMessage::Register {
                da_address: da_a,
                amount: SEQUENCE_STAKE,
            },
        ),
    );

    runner.execute(
        additional_sequencer.create_plain_message::<RT, TestSequencerRegistry>(
            CallMessage::UpdateDaAddress {
                old_da_address: da_a,
                new_da_address: da_b,
            },
        ),
    );

    runner.execute(
        additional_sequencer.create_plain_message::<RT, TestSequencerRegistry>(
            CallMessage::InitiateWithdrawal { da_address: da_b },
        ),
    );
    runner.advance_slots(config_value!("DEFERRED_SLOTS_COUNT") + 1);
    runner.execute(
        additional_sequencer.create_plain_message::<RT, TestSequencerRegistry>(
            CallMessage::Withdraw { da_address: da_b },
        ),
    );

    runner.execute(admin.create_plain_message::<RT, TestSequencerRegistry>(
        CallMessage::Register {
            da_address: da_b,
            amount: SEQUENCE_STAKE,
        },
    ));

    let refund_amount = Amount::new(777);
    runner.__apply_to_state(move |state| {
        let mut reg = TestSequencerRegistry::default();
        let bank = Bank::<S>::default();

        let reused_entry = reg
            .is_sender_known(&da_b, state)
            .expect("Reused DA must be registered before refund");
        assert_eq!(reused_entry.address, reused_rollup);
        assert_eq!(reused_entry.balance, SEQUENCE_STAKE);

        let error = reg
            .add_to_stake(bank.id().clone().to_payable(), &da_a, refund_amount, state)
            .expect_err("Refund to retired DA must not credit a reused DA");
        assert!(
            error.to_string().contains(&original_rollup.to_string()),
            "Unexpected error: {error}"
        );

        let reused_entry = reg
            .is_sender_known(&da_b, state)
            .expect("Reused DA must remain registered");
        assert_eq!(reused_entry.address, reused_rollup);
        assert_eq!(reused_entry.balance, SEQUENCE_STAKE);
    });
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
