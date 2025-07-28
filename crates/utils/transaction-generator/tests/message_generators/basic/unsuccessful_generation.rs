use sov_modules_api::prelude::tokio;
use sov_test_utils::runtime::TestRunner;
use sov_test_utils::{TestSpec as S, TransactionTestCase, TransactionType};
use sov_transaction_generator::interface::{MessageValidity, Percent};
use sov_transaction_generator::Distribution;

use super::ModulesToUse;
use crate::basic::{test_with_modules, GeneratorOutput, RT, TXS_TO_GENERATE};
use crate::{NumTxsExecuted, TestRuntimeCall};

fn test_reverted_transactions_helper(modules: Distribution<ModulesToUse>) -> NumTxsExecuted {
    let mut transaction_exec_closure =
        move |tx: TransactionType<RT, S>,
              expected_output: GeneratorOutput,
              runner: &mut TestRunner<RT, S>| {
            runner.execute_transaction(TransactionTestCase {
                input: tx,
                assert: Box::new(move |receipt, _state| {
                    assert_eq!(
                        expected_output.outcome.clone().unwrap_changes().len(),
                        0,
                        "There shouldn't be any change to the state. Expected output {expected_output:?}"
                    );
                    assert!(
                        receipt.tx_receipt.is_reverted(),
                        "The transaction should be reverted. Instead, got hte receipt {:?}", receipt.tx_receipt
                    );
                }),
            });
        };

    let (_, outputs) = test_with_modules(
        modules,
        MessageValidity::as_distribution(Percent::zero()),
        &mut transaction_exec_closure,
    );

    let (num_value_setter_txs, num_access_pattern_txs, num_bank_txs) = outputs.iter().fold(
        (0, 0, 0),
        |(num_value_setter_txs, num_access_pattern_txs, num_bank_txs), output| match output.message
        {
            TestRuntimeCall::Bank(_) => (
                num_value_setter_txs,
                num_access_pattern_txs,
                num_bank_txs + 1,
            ),
            TestRuntimeCall::AccessPattern(_) => (
                num_value_setter_txs,
                num_access_pattern_txs + 1,
                num_bank_txs,
            ),
            TestRuntimeCall::ValueSetter(_) => (
                num_value_setter_txs + 1,
                num_access_pattern_txs,
                num_bank_txs,
            ),
            _ => panic!("Unexpected message type"),
        },
    );

    NumTxsExecuted {
        num_value_setter_txs,
        num_access_pattern_txs,
        num_bank_txs,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_reverted_transactions_only_bank() {
    let NumTxsExecuted {
        num_bank_txs,
        num_access_pattern_txs,
        num_value_setter_txs,
    } = test_reverted_transactions_helper(Distribution::with_equiprobable_values(vec![
        ModulesToUse::Bank,
    ]));
    assert_eq!(
        num_bank_txs, TXS_TO_GENERATE,
        "Not enough bank txs generated: generated {num_bank_txs}, expected {TXS_TO_GENERATE}"
    );
    assert_eq!(
        num_value_setter_txs, 0,
        "Too many value setter txs generated: generated {num_value_setter_txs}, expected 0"
    );
    assert_eq!(
        num_access_pattern_txs, 0,
        "Too many access pattern txs generated: generated {num_access_pattern_txs}, expected 0"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_reverted_transactions_only_value_setter() {
    let NumTxsExecuted {
        num_bank_txs,
        num_access_pattern_txs,
        num_value_setter_txs,
    } = test_reverted_transactions_helper(Distribution::with_equiprobable_values(vec![
        ModulesToUse::ValueSetter,
    ]));
    assert_eq!(
        num_value_setter_txs, TXS_TO_GENERATE,
        "Not enough value setter txs generated: generated {num_value_setter_txs}, expected {TXS_TO_GENERATE}"
    );
    assert_eq!(
        num_bank_txs, 0,
        "Too many bank txs generated: generated {num_bank_txs}, expected 0"
    );
    assert_eq!(
        num_access_pattern_txs, 0,
        "Too many access pattern txs generated: generated {num_access_pattern_txs}, expected 0"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_reverted_transactions_only_access_patterns() {
    let NumTxsExecuted {
        num_bank_txs,
        num_access_pattern_txs,
        num_value_setter_txs,
    } = test_reverted_transactions_helper(Distribution::with_equiprobable_values(vec![
        ModulesToUse::AccessPattern,
    ]));
    assert_eq!(
        num_access_pattern_txs, TXS_TO_GENERATE,
        "Not enough access pattern txs generated: generated {num_access_pattern_txs}, expected {TXS_TO_GENERATE}"
    );
    assert_eq!(
        num_bank_txs, 0,
        "Too many bank txs generated: generated {num_bank_txs}, expected 0"
    );
    assert_eq!(
        num_value_setter_txs, 0,
        "Too many value setter txs generated: generated {num_value_setter_txs}, expected 0"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_reverted_transactions_combined() {
    let NumTxsExecuted {
        num_bank_txs,
        num_access_pattern_txs,
        num_value_setter_txs,
    } = test_reverted_transactions_helper(Distribution::with_equiprobable_values(vec![
        ModulesToUse::ValueSetter,
        ModulesToUse::AccessPattern,
        ModulesToUse::Bank,
    ]));
    assert!(
        num_value_setter_txs > 0,
        "Not enough value setter txs generated: generated {num_value_setter_txs}, expected at least 1"
    );
    assert!(
        num_access_pattern_txs > 0,
        "Not enough access pattern txs generated: generated {num_access_pattern_txs}, expected at least 1"
    );
    assert!(
        num_bank_txs > 0,
        "Not enough bank txs generated: generated {num_bank_txs}, expected at least 1"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_reverted_transaction_generation_mixed() {
    let NumTxsExecuted {
        num_bank_txs,
        num_access_pattern_txs,
        num_value_setter_txs,
    } = test_reverted_transactions_helper(Distribution::with_values(vec![
        (6, ModulesToUse::Bank),
        (2, ModulesToUse::AccessPattern),
        (2, ModulesToUse::ValueSetter),
    ]));

    // We should have generated at least one transaction of each module
    assert!(num_bank_txs > 0);
    assert!(num_access_pattern_txs > 0);
    assert!(num_value_setter_txs > 0);
    assert!(
        num_bank_txs > 2 * num_value_setter_txs,
        "There should be at more bank transactions generated"
    );
    assert!(
        num_bank_txs > 2 * num_access_pattern_txs,
        "There should be at more bank transactions generated"
    );
}
