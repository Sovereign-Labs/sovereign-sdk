use std::cell::Cell;
use std::rc::Rc;

use sov_modules_api::{Amount, Spec, TxEffect};
use sov_test_utils::{AsUser, TransactionTestCase};

use crate::helpers::{setup, TestAttesterIncentives, RT, S};

/// The gas cost of one state read in this test. It is much larger than the cost of all the other
/// operations of the transaction together, so a gas limit of `n * READ_GAS + READ_GAS / 2` lets
/// the transaction do exactly `n` state reads.
const READ_GAS: u64 = 1_000_000;

/// An upper bound on the number of state reads of the transaction.
const MAX_READS: u64 = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Skipped,
    Reverted,
    Successful,
}

/// Regression test: a transaction that runs out of gas on a state read of the call must be
/// reverted. Previously the module panicked when the operating mode read ran out of gas.
#[test]
fn test_call_reverts_when_out_of_gas_on_state_read() {
    std::env::set_var(
        "SOV_TEST_CONST_OVERRIDE_GAS_TO_CHARGE_PER_READ",
        format!("[{READ_GAS}, {READ_GAS}]"),
    );
    let (mut runner, attester, _, _) = setup();

    // Run out of gas on each state read of the transaction in turn.
    let mut outcomes = Vec::new();
    for affordable_reads in 0..MAX_READS {
        let gas_limit = affordable_reads * READ_GAS + READ_GAS / 2;
        let outcome = Rc::new(Cell::new(None));
        let outcome_ref = outcome.clone();

        runner.execute_transaction(TransactionTestCase {
            input: attester
                .create_plain_message::<RT, TestAttesterIncentives>(
                    sov_attester_incentives::CallMessage::DepositAttester(Amount::ZERO),
                )
                .with_gas_limit(Some(<S as Spec>::Gas::from([gas_limit; 2]))),
            assert: Box::new(move |result, _state| {
                outcome_ref.set(Some(match &result.tx_receipt {
                    TxEffect::Skipped(_) => Outcome::Skipped,
                    TxEffect::Reverted(_) => Outcome::Reverted,
                    TxEffect::Successful(_) => Outcome::Successful,
                }));
            }),
        });

        let outcome = outcome.get().expect("The transaction must have a receipt");
        outcomes.push(outcome);
        if outcome == Outcome::Successful {
            break;
        }
    }

    assert_eq!(
        outcomes.last(),
        Some(&Outcome::Successful),
        "The transaction should succeed when it can pay for all its state reads"
    );
    assert!(
        outcomes.contains(&Outcome::Reverted),
        "The transaction should revert when it runs out of gas on a state read of the call, got {outcomes:?}"
    );
}
