use sov_bank::utils::TokenHolder;
use sov_bank::{config_gas_token_id, Bank};
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::transaction::PriorityFeeBips;
use sov_modules_api::{Amount, Gas, GasArray, GasSpec, GetGasPrice, ModuleInfo};
use sov_sequencer_registry::SequencerRegistry;
use sov_test_utils::runtime::TestRunner;
use sov_test_utils::{
    assert_matches, get_gas_used, AsUser, BatchTestCase, TestUser, TransactionTestCase,
    TransactionType, TxProcessingError,
};

use super::helpers::S;
use crate::helpers::{setup, TestRoles, RT};

const VALUE_SETTER_NEW_CONST: u32 = 10;
const OTHER_VALUE_SETTER_CONST: u32 = 42;

/// Initialize the reward mechanism tests, and executes an empty slot to know how much gas is consumed by a simple value setter transaction.
fn reward_mechanism_test_setup() -> (TestRoles, Amount, TestRunner<RT, S>) {
    // Genesis initialization.
    // We need to pass the large balance to make sure we have enough funds to pay for the tip and the sequencer registration
    let (test_roles, mut runner) = setup();

    let default_sequencer = &test_roles.default_sequencer;
    let admin = &test_roles.admin;

    runner.config.sequencer_da_address = default_sequencer.da_address;
    // We first execute a normal transaction with no priority fee (ie the sequencer does not get rewarded).
    // This way we can know how much gas was consumed. Check that the sequencer balance was not updated
    let (output, _, _) = runner.simulate(
        admin
            .create_plain_message::<RT, sov_value_setter::ValueSetter<S>>(
                sov_value_setter::CallMessage::SetValue {
                    value: VALUE_SETTER_NEW_CONST,
                    gas: None,
                },
            )
            .with_max_priority_fee_bips(PriorityFeeBips::ZERO),
    );

    let gas_consumed_last_tx = get_gas_used(
        output
            .batch_receipts
            .last()
            .unwrap()
            .tx_receipts
            .last()
            .unwrap(),
    );
    let initial_gas_price = S::initial_base_fee_per_gas();

    (
        test_roles,
        gas_consumed_last_tx.value(&initial_gas_price),
        runner,
    )
}

fn reward_mechanism_test(
    max_fee: Amount,
    max_priority_fee: PriorityFeeBips,
    expected_reward: Amount,
    roles: TestRoles,
    runner: &mut TestRunner<RT, S>,
) {
    let TestRoles {
        default_sequencer: test_sequencer,
        admin,
        ..
    } = roles;

    let test_sequencer_da_address = test_sequencer.da_address;
    let test_sequencer_bond = test_sequencer.bond;

    runner.execute_transaction(TransactionTestCase {
        input: admin
            .create_plain_message::<RT, sov_value_setter::ValueSetter<S>>(
                sov_value_setter::CallMessage::SetValue {
                    value: OTHER_VALUE_SETTER_CONST,
                    gas: None,
                },
            )
            .with_max_fee(max_fee)
            .with_max_priority_fee_bips(max_priority_fee),
        assert: Box::new(move |result, state| {
            let gas_price = state.gas_price();
            let sequencer_burn = S::gas_to_charge_per_byte_borsh_deserialization()
                .checked_scalar_product(result.blob_info.size as u64)
                .unwrap()
                .checked_value(gas_price)
                .unwrap();
            let expected_sequencer_balance = test_sequencer_bond
                .checked_add(expected_reward)
                .unwrap()
                .checked_sub(sequencer_burn)
                .unwrap();
            assert_eq!(
                TestRunner::<RT, S>::get_sequencer_staking_balance(
                    &test_sequencer_da_address,
                    state
                ),
                Some(expected_sequencer_balance),
                "The sequencer was not rewarded the correct amount"
            );
        }),
    });
}

/// Tests that the sequencer gets rewarded some gas following the EIP-1559 rules.
/// When the `max_fee` is high enough and the batch is successfully executed, the sequencer gets the `consumed_gas * priority_fee`
#[test]
fn test_reward_sequencer_max_fee_high_enough() {
    let (roles, gas_consumed, mut runner) = reward_mechanism_test_setup();

    let priority_fee = PriorityFeeBips::from_percentage(10);

    let expected_reward = priority_fee.apply(gas_consumed).unwrap();
    let max_fee = gas_consumed.checked_add(expected_reward).unwrap();

    reward_mechanism_test(max_fee, priority_fee, expected_reward, roles, &mut runner);
}

/// Tests that the sequencer gets rewarded some gas following the EIP-1559 rules.
/// When the `max_fee` is high enough to only pay for the transaction execution costs and the batch is successfully executed, the sequencer does
/// not get rewarded.
#[test]
fn test_reward_sequencer_max_fee_not_high_enough() {
    let (roles, gas_consumed, mut runner) = reward_mechanism_test_setup();

    let priority_fee = PriorityFeeBips::from_percentage(10);

    let expected_reward = Amount::ZERO;
    let max_fee = gas_consumed;

    reward_mechanism_test(max_fee, priority_fee, expected_reward, roles, &mut runner);
}

/// Tests that the sequencer registry balance accumulates the sequencer's rewards.
#[test]
fn test_reward_sequencer_registry() {
    let (roles, gas_consumed, mut runner) = reward_mechanism_test_setup();

    let sequencer_registry_balance = |runner: &TestRunner<RT, S>| {
        runner.query_visible_state(|state| {
            let sequencer_id = *SequencerRegistry::<S>::default().id();

            Bank::<S>::default()
                .get_balance_of(
                    TokenHolder::Module(sequencer_id),
                    config_gas_token_id(),
                    state,
                )
                .unwrap_infallible()
        })
    };

    let priority_fee = PriorityFeeBips::from_percentage(10);

    let expected_reward = priority_fee.apply(gas_consumed).unwrap();
    let max_fee = gas_consumed.checked_add(expected_reward).unwrap();

    let balance_before = sequencer_registry_balance(&runner).unwrap();

    reward_mechanism_test(max_fee, priority_fee, expected_reward, roles, &mut runner);

    let balance_after = sequencer_registry_balance(&runner).unwrap();

    assert_eq!(
        balance_before.checked_add(expected_reward).unwrap(),
        balance_after,
        "The sequencer registry balance should increase after rewarding the sequencer"
    );
}

/// Tests that the sequencer gets correctly penalized when it incorrectly processes a batch
/// For instance, this happens if there is no enough gas to execute a transaction in a batch.
#[test]
fn test_penalize_sequencer() {
    let (
        TestRoles {
            default_sequencer,
            admin,
            ..
        },
        mut runner,
    ) = setup();

    let default_sequencer_stake = default_sequencer.bond;
    let default_sequencer_da_address = default_sequencer.da_address;

    runner.execute_transaction(TransactionTestCase {
        input: admin
            .create_plain_message::<RT, sov_value_setter::ValueSetter<S>>(
                sov_value_setter::CallMessage::SetValue{value:OTHER_VALUE_SETTER_CONST,gas: None},
            )
            .with_max_fee(Amount::ZERO),
        assert: Box::new(move |result, state| {
            match &result.tx_receipt {
                sov_modules_api::TxEffect::Skipped(skipped) => {
                    if let TxProcessingError::OutOfGas(error_message) = &skipped.error {
                        assert!(error_message.contains("The amount to charge is greater than the funds available in the meter."), "Error message doesn't contain with the expected phrase. Got: {error_message}");
                    } else {
                        panic!("Expected CannotReserveGas error, but got a different SkippedReason: {:?}", skipped.error);
                    }
                },
                unexpected => panic!("Expected transaction to be skipped, but got: {unexpected:?}"),
            }

            let current_stake = sov_sequencer_registry::SequencerRegistry::<S>::default()
                .get_sender_balance_via_api(&default_sequencer_da_address, state)
                .unwrap();
            let genesis_stake = default_sequencer_stake;

            assert!(
                current_stake < default_sequencer_stake,
                "The sequencer stake has not decreased which means he wasn't penalized: current stake {current_stake}, genesis stake {genesis_stake}"
            );
        })
    });
}

fn produce_malformed_tx(
    runner: &mut TestRunner<RT, S>,
    admin: &TestUser<S>,
) -> TransactionType<RT, S> {
    let mut nonces = runner.nonces().clone();

    let mut tx = admin
        .create_plain_message::<RT, sov_value_setter::ValueSetter<S>>(
            sov_value_setter::CallMessage::SetValue {
                value: 10,
                gas: None,
            },
        )
        .to_serialized_authenticated_tx(&mut nonces);

    tx.data.pop();
    TransactionType::PreAuthenticated(tx)
}

#[test]
fn test_authentication_out_of_gas_error() {
    let (
        TestRoles {
            admin,
            default_sequencer,
            ..
        },
        mut runner,
    ) = setup();

    let seq_address = default_sequencer.da_address;
    let initial_seq_bond = default_sequencer.bond;
    let malformed_transaction = produce_malformed_tx(&mut runner, &admin);

    // First, we send a transaction with max fee 0. Since the tx doesn't provide enough fees to cover
    // the cost of its deserialization, the sequencer pays the difference. Second tx is malformed.
    runner.execute_batch(BatchTestCase {
        input: vec![
            admin
                .create_plain_message::<RT, sov_value_setter::ValueSetter<S>>(
                    sov_value_setter::CallMessage::SetValue{value:10,gas: None},
                )
                .with_max_fee(Amount::ZERO),
            malformed_transaction,
        ]
        .into(),
        assert: Box::new(move |result, state| {
            let batch_receipt = result.batch_receipt.as_ref().unwrap();
            assert_eq!(
                batch_receipt.tx_receipts.len(), 2,
                "Only the first transaction should have been included in the batch"
            );

            let tx_receipt = &batch_receipt.tx_receipts[0];
            match &tx_receipt.receipt {
                sov_modules_api::TxEffect::Skipped(skipped) => {
                    if let TxProcessingError::OutOfGas(error_message) = &skipped.error {
                        assert!(
                            error_message.contains("The amount to charge is greater than the funds available in the meter."),
                            "Error message doesn't contain with the expected phrase. Got: {error_message}"
                        );
                    } else {
                        panic!("Expected CannotReserveGas error, but got a different SkippedReason: {:?}", skipped.error);
                    }
                },
                unexpected => panic!("Expected transaction to revert, but got: {unexpected:?}"),
            };

            let tx_receipt = &batch_receipt.tx_receipts[1];
            match &tx_receipt.receipt {
                sov_modules_api::TxEffect::Skipped(skipped) => {
                    assert_matches!(skipped.error, TxProcessingError::AuthenticationFailed(_));
                },
                unexpected => panic!("Expected transaction to revert, but got: {unexpected:?}"),
            };

            // Check that sequencer was penalized for invalid transactions.
            let bond = TestRunner::<RT, S>::get_sequencer_staking_balance(
                &seq_address,
                state
            ).unwrap();
            assert!(bond < initial_seq_bond);
        }),
    });
}
