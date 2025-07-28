use std::collections::HashMap;

use sov_bank::{config_gas_token_id, Bank};
use sov_modules_api::macros::config_value;
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::registration_lib::StakeRegistration;
use sov_modules_api::{Amount, Gas, GasArray, GetGasPrice, Spec, TxEffect};
use sov_prover_incentives::{CallMessage, Event, ProverIncentives};
use sov_test_utils::{AsUser, BatchTestCase, BatchType, TransactionTestCase, TransactionType};
use sov_value_setter::ValueSetter;

use crate::helpers::{
    minimal_bond, setup, setup_with_custom_runtime, TestProverIncentives, TestRuntimeEvent, RT,
};

pub(crate) type S = sov_test_utils::TestSpec;

#[test]
fn test_genesis_bond() {
    let (runner, genesis_prover, _) = setup();

    runner.query_visible_state(|state| {
        assert_eq!(
            TestProverIncentives::default()
                .bonded_provers
                .get(&genesis_prover.user_info.address(), state)
                .unwrap(),
            Some(genesis_prover.bond),
            "The genesis prover should be bonded"
        );
        assert_eq!(
            Bank::<S>::default()
                .get_balance_of(
                    &genesis_prover.user_info.address(),
                    config_gas_token_id(),
                    state
                )
                .unwrap_infallible(),
            Some(genesis_prover.user_info.available_gas_balance),
            "The balance of the prover should be equal to the free balance"
        );
    });
}

#[test]
fn test_topup_existing_bond() {
    let (mut runner, genesis_prover, _) = setup();

    let starting_free_balance = genesis_prover.user_info.balance();
    let starting_bond = genesis_prover.bond;
    let extra_bond_amount = Amount::new(50);
    let prover_address = genesis_prover.user_info.address();

    let test = TransactionTestCase::<RT, S> {
        input: genesis_prover.create_plain_message::<RT, TestProverIncentives>(
            CallMessage::Deposit(extra_bond_amount),
        ),
        assert: Box::new(move |result, state| {
            assert!(result.events.iter().any(|event| matches!(
                event,
                TestRuntimeEvent::ProverIncentives(Event::Deposited {
                    prover,
                    deposit
                }) if *prover == prover_address && *deposit == extra_bond_amount
            )));
            assert_eq!(
                TestProverIncentives::default()
                    .bonded_provers
                    .get(&prover_address, state)
                    .unwrap(),
                Some(starting_bond.checked_add(extra_bond_amount).unwrap()),
            );
            assert_eq!(
                Bank::<S>::default()
                    .get_balance_of(&prover_address, config_gas_token_id(), state)
                    .unwrap_infallible(),
                Some(
                    starting_free_balance
                        .checked_sub(extra_bond_amount)
                        .unwrap()
                        .checked_sub(result.gas_value_used)
                        .unwrap()
                ),
            );
        }),
    };

    runner.execute_transaction(test);
}

// Note: we are bonding less than `minimum_bond` amount, currently this is allowed
// as users are able to deposit more bond and we check the user is sufficiently
// bonded when processing submitted proofs.
#[test]
fn test_bonding_new_prover() {
    let (mut runner, _, unbonded_user) = setup();

    let starting_free_balance = unbonded_user.balance();
    let bond_amount = Amount::new(100000001);
    let user_address = unbonded_user.address();

    runner.execute_transaction(TransactionTestCase {
        input: unbonded_user
            .create_plain_message::<RT, TestProverIncentives>(CallMessage::Register(bond_amount)),
        assert: Box::new(move |result, state| {
            assert!(result.events.iter().any(|event| matches!(
                event,
                TestRuntimeEvent::ProverIncentives(Event::Registered {
                    prover,
                    amount
                }) if *prover == user_address && *amount == bond_amount
            )));
            assert_eq!(
                TestProverIncentives::default()
                    .bonded_provers
                    .get(&unbonded_user.address(), state)
                    .unwrap(),
                Some(bond_amount),
            );
            assert_eq!(
                Bank::<S>::default()
                    .get_balance_of(&unbonded_user.address(), config_gas_token_id(), state)
                    .unwrap_infallible(),
                Some(
                    starting_free_balance
                        .checked_sub(bond_amount)
                        .unwrap()
                        .checked_sub(result.gas_value_used)
                        .unwrap()
                ),
            );
        }),
    });
}

#[test]
fn test_unbonding() {
    let (mut runner, genesis_prover, _) = setup();

    runner.execute_transaction(TransactionTestCase {
        input: genesis_prover.create_plain_message::<RT, TestProverIncentives>(CallMessage::Exit),
        assert: Box::new(move |result, state| {
            assert!(result.events.iter().any(|event| matches!(
                event,
                TestRuntimeEvent::ProverIncentives(Event::Exited { .. })
            )));
            assert_eq!(
                Bank::<S>::default()
                    .get_balance_of(
                        &genesis_prover.user_info.address(),
                        config_gas_token_id(),
                        state
                    )
                    .unwrap_infallible()
                    .unwrap(),
                genesis_prover
                    .user_info
                    .available_gas_balance
                    .checked_add(genesis_prover.bond)
                    .unwrap()
                    .checked_sub(result.gas_value_used)
                    .unwrap()
            );
        }),
    });
}

/// This test ensures that the prover cannot send proofs when the gas price is too high.
/// That is, if the gas price increases, the prover will not have bonded enough funds and then won't be able to prove anymore.
/// Currently, the easiest way to do this is to artificially change the gas cost of some operation in the bank module. We do that
/// by modifying the runtime manually.
#[test]
fn test_cannot_prove_when_gas_price_is_too_high() {
    let mut gas_limit = <S as Spec>::Gas::from(config_value!("INITIAL_GAS_LIMIT"));
    let gas_target = gas_limit.scalar_division(2).clone();

    let runtime = RT::default();

    let (mut runner, prover, unbonded_user) = setup_with_custom_runtime(runtime);

    let mut nonces = HashMap::new();

    let additional_prover_bond = minimal_bond(&runner);

    let initial_gas_price = runner.query_visible_state(|state| state.gas_price().clone());

    let bank_signed = prover
        .create_plain_message::<RT, ValueSetter<S>>(sov_value_setter::CallMessage::SetValue {
            value: 1,
            gas: Some(gas_target.clone()),
        })
        .with_max_fee(
            prover
                .user_info
                .available_gas_balance
                .checked_div(Amount::new(2))
                .unwrap(),
        )
        .to_serialized_authenticated_tx(&mut nonces);

    let register_signed = unbonded_user
        .create_plain_message::<RT, ProverIncentives<S>>(CallMessage::Register(
            additional_prover_bond.into(),
        ))
        .to_serialized_authenticated_tx(&mut nonces);

    // We execute a batch of two transactions, check that the total gas used is higher than the target.
    runner.execute_batch(BatchTestCase {
        input: BatchType(vec![
            TransactionType::<RT, S>::PreAuthenticated(bank_signed),
            TransactionType::<RT, S>::PreAuthenticated(register_signed),
        ]),
        assert: Box::new(move |result, _state| {
            assert_eq!(result.batch_receipt.clone().unwrap().tx_receipts.len(), 2);

            let mut total_gas_used = <S as Spec>::Gas::zero();

            for (i, tx_receipt) in result.batch_receipt.unwrap().tx_receipts.iter().enumerate() {
                match &tx_receipt.receipt {
                    TxEffect::Successful(tx_contents) => {
                        total_gas_used = total_gas_used
                            .checked_combine(&tx_contents.gas_used)
                            .unwrap();
                    }
                    _ => {
                        panic!("Tx {i} with receipt {tx_receipt:?} should be successful");
                    }
                }
            }

            assert!(
                gas_target.dim_is_less_than(&total_gas_used),
                "The total gas used should be higher than the initial gas used"
            );
        }),
    });

    // We need to advance by one slot to update the gas price
    runner.advance_slots(1);

    let new_bond_amount = minimal_bond(&runner);

    runner.query_visible_state(|state| {
        let new_gas_price = state.gas_price();

        assert!(
            initial_gas_price.dim_is_less_than(new_gas_price),
            "The new gas price {new_gas_price} should be higher than the initial gas price {initial_gas_price}"
        );

        assert!(
            new_bond_amount > additional_prover_bond,
            "The new bond amount {new_bond_amount} should be higher than the initial additional prover bond {additional_prover_bond}."
        );

        let prover = ProverIncentives::<S>::default().get_allowed_staker(&unbonded_user.address(), state).unwrap_infallible();

        // The prover should be registered
        assert!(prover.is_some(), "The additional prover should be registered");

        // But he should not be allowed to send transactions because he doesn't have enough stake.
        assert!(
            prover.unwrap().1 < new_bond_amount,
            "The prover should not be allowed to send transactions because he doesn't have enough stake."
        );
    });
}
