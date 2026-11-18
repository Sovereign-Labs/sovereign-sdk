use std::cell::RefCell;
use std::rc::Rc;

use sov_mock_da::MockAddress;
use sov_modules_api::macros::config_value;
use sov_modules_api::{Amount, Spec, TxEffect};
use sov_test_utils::{AsUser, TransactionTestCase};

use crate::helpers::{setup, TestSequencerRegistry, NON_DEFAULT_SEQUENCER_DA_ADDRESS, RT, S};

const SEQUENCER_STAKE: Amount = Amount::new(100_000_000_000);

/// The gas cost of one state read in this test. It is much larger than the cost of all the other
/// operations of the transaction together, so a gas limit of `n * READ_GAS + READ_GAS / 2` lets
/// the transaction do exactly `n` state reads.
const READ_GAS: u64 = 1_000_000;

/// An upper bound on the number of state reads of the transaction.
const MAX_READS: u64 = 64;

/// Regression test: a withdrawal that runs out of gas during the transfer of the staked balance
/// must be reverted with an out-of-gas error. Previously the module panicked.
#[test]
fn test_withdraw_reverts_when_out_of_gas_during_transfer() {
    std::env::set_var(
        "SOV_TEST_CONST_OVERRIDE_GAS_TO_CHARGE_PER_READ",
        format!("[{READ_GAS}, {READ_GAS}]"),
    );
    // Keep the withdrawal delay short, so the test does not have to run many empty slots.
    std::env::set_var("SOV_TEST_CONST_OVERRIDE_DEFERRED_SLOTS_COUNT", "1");
    let (roles, mut runner) = setup();
    let sequencer = roles.additional_sequencer;
    let da_address = MockAddress::new(NON_DEFAULT_SEQUENCER_DA_ADDRESS);

    runner.execute_transaction(TransactionTestCase {
        input: sequencer.create_plain_message::<RT, TestSequencerRegistry>(
            sov_sequencer_registry::CallMessage::Register {
                da_address,
                amount: SEQUENCER_STAKE,
            },
        ),
        assert: Box::new(move |result, _state| {
            assert!(result.tx_receipt.is_successful(), "Registration failed");
        }),
    });
    runner.execute_transaction(TransactionTestCase {
        input: sequencer.create_plain_message::<RT, TestSequencerRegistry>(
            sov_sequencer_registry::CallMessage::InitiateWithdrawal { da_address },
        ),
        assert: Box::new(move |result, _state| {
            assert!(
                result.tx_receipt.is_successful(),
                "Withdrawal initiation failed"
            );
        }),
    });
    runner.advance_slots(config_value!("DEFERRED_SLOTS_COUNT") + 1);

    // Run out of gas on each state read of the withdrawal in turn.
    let mut revert_reasons = Vec::new();
    let mut withdrawn = false;
    for affordable_reads in 0..MAX_READS {
        let gas_limit = affordable_reads * READ_GAS + READ_GAS / 2;
        let receipt = Rc::new(RefCell::new(None));
        let receipt_ref = receipt.clone();

        runner.execute_transaction(TransactionTestCase {
            input: sequencer
                .create_plain_message::<RT, TestSequencerRegistry>(
                    sov_sequencer_registry::CallMessage::Withdraw { da_address },
                )
                .with_gas_limit(Some(<S as Spec>::Gas::from([gas_limit; 2]))),
            assert: Box::new(move |result, _state| {
                *receipt_ref.borrow_mut() = Some(result.tx_receipt.clone());
            }),
        });

        match receipt.take().expect("The transaction must have a receipt") {
            TxEffect::Skipped(_) => {}
            TxEffect::Reverted(reverted) => revert_reasons.push(reverted.reason.to_string()),
            TxEffect::Successful(_) => {
                withdrawn = true;
                break;
            }
        }
    }

    assert!(
        withdrawn,
        "The withdrawal should succeed when it can pay for all its state reads"
    );
    assert!(
        revert_reasons.iter().any(|reason| reason
            .contains("could not withdraw because the transaction ran out of gas")),
        "The withdrawal should revert with an out-of-gas error when the transfer runs out of gas, got {revert_reasons:#?}"
    );
}
