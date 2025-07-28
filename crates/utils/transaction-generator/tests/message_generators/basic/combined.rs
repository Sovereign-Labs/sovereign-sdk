use std::sync::Arc;

use sov_modules_api::prelude::tokio;
use sov_test_utils::runtime::TestRunner;
use sov_test_utils::{TestSpec as S, TransactionTestCase, TransactionType};
use sov_transaction_generator::generators::basic::BasicClientConfig;
use sov_transaction_generator::interface::MessageValidity;
use sov_transaction_generator::{assert_logs_against_state, Distribution, Percent};

use super::{test_with_modules, GeneratorOutput, ModulesToUse, TXS_TO_GENERATE};
use crate::basic::RT;
use crate::{NumTxsExecuted, TestRuntimeCall};

async fn test_combined_generation_helper(modules: Distribution<ModulesToUse>) -> NumTxsExecuted {
    let mut transaction_exec_closure =
        |tx: TransactionType<RT, S>, output: GeneratorOutput, runner: &mut TestRunner<RT, S>| {
            runner.execute_transaction(TransactionTestCase {
                input: tx.clone(),
                assert: Box::new(move |result, _state| {
                    // If we expect to have at least one change on the state, the transaction should be successful
                    if output.outcome.is_successful() {
                        assert!(result.tx_receipt.is_successful(), "{:?}", result.tx_receipt);
                    } else {
                        assert!(
                            result.tx_receipt.is_reverted(),
                            "Receipt: {:?}, tx: {:?}",
                            result.tx_receipt,
                            tx
                        );
                    }
                }),
            });
        };

    let (mut runner, outputs) = test_with_modules(
        modules,
        MessageValidity::as_distribution(Percent::fifty()),
        &mut transaction_exec_closure,
    );

    let _ = runner.setup_rest_api_server().await;
    let config = BasicClientConfig {
        url: runner.base_path(),
        rollup_height: None,
    };

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

    // We also assert the changes against the state if there is any positive changes.
    let changes = outputs
        .into_iter()
        .flat_map(|output| output.outcome.unwrap_changes())
        .collect();

    assert_logs_against_state(changes, Arc::new(config), 1)
        .await
        .expect("Failed to assert against state");

    NumTxsExecuted {
        num_bank_txs,
        num_access_pattern_txs,
        num_value_setter_txs,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_combined_transaction_generation() {
    let NumTxsExecuted {
        num_bank_txs,
        num_access_pattern_txs,
        num_value_setter_txs,
    } = test_combined_generation_helper(Distribution::with_equiprobable_values(vec![
        ModulesToUse::Bank,
        ModulesToUse::AccessPattern,
        ModulesToUse::ValueSetter,
    ]))
    .await;

    // We should have generated at least one bank and one value setter tx
    assert!(num_bank_txs > 0);
    assert!(num_access_pattern_txs > 0);
    assert!(num_value_setter_txs > 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_combined_transaction_generation_mixed() {
    let NumTxsExecuted {
        num_bank_txs,
        num_access_pattern_txs,
        num_value_setter_txs,
    } = test_combined_generation_helper(Distribution::with_values(vec![
        (6, ModulesToUse::Bank),
        (2, ModulesToUse::AccessPattern),
        (2, ModulesToUse::ValueSetter),
    ]))
    .await;

    // We should have generated at least one bank and one value setter tx
    assert!(num_bank_txs > 0);
    assert!(num_value_setter_txs > 0);
    assert!(num_access_pattern_txs > 0);
    assert!(
        num_bank_txs > 2 * num_value_setter_txs,
        "There should be at more bank transactions generated"
    );
    assert!(
        num_bank_txs > 2 * num_access_pattern_txs,
        "There should be at more bank transactions generated"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_generate_txs_only_value_setter() {
    let NumTxsExecuted {
        num_bank_txs,
        num_access_pattern_txs,
        num_value_setter_txs,
    } = test_combined_generation_helper(Distribution::with_equiprobable_values(vec![
        ModulesToUse::ValueSetter,
    ]))
    .await;

    // We should have generated zero bank transaction and 100 value setter transactions
    assert_eq!(num_bank_txs, 0);
    assert_eq!(num_access_pattern_txs, 0);
    assert_eq!(num_value_setter_txs, TXS_TO_GENERATE);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_generate_txs_only_bank() {
    let NumTxsExecuted {
        num_bank_txs,
        num_access_pattern_txs,
        num_value_setter_txs,
    } = test_combined_generation_helper(Distribution::with_equiprobable_values(vec![
        ModulesToUse::Bank,
    ]))
    .await;

    // We should have generated zero value setter transaction and 100 bank transactions
    assert_eq!(num_bank_txs, TXS_TO_GENERATE);
    assert_eq!(num_value_setter_txs, 0);
    assert_eq!(num_access_pattern_txs, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_generate_txs_only_access_pattern() {
    let NumTxsExecuted {
        num_bank_txs,
        num_access_pattern_txs,
        num_value_setter_txs,
    } = test_combined_generation_helper(Distribution::with_equiprobable_values(vec![
        ModulesToUse::AccessPattern,
    ]))
    .await;

    // We should have generated zero value setter transaction and 100 bank transactions
    assert_eq!(num_access_pattern_txs, TXS_TO_GENERATE);
    assert_eq!(num_value_setter_txs, 0);
    assert_eq!(num_bank_txs, 0);
}
