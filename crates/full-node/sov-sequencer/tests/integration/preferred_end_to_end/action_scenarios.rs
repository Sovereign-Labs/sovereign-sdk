use super::*;

// FIXME(@neysofu): this test is not broken due to correctness bugs in the
// sequencer, but rather because generated testing scenarios sometimes are
// oversized and the node can't keep up with the sequencer. TODO: find a solution.
//#[test]
//fn random_edge_cases_and_complex_scenarios() {
//    use proptest::prelude::*;
//    use proptest::test_runner::{Config, TestRunner};
//
//    let mut runner = TestRunner::new(Config::with_cases(1));
//    let result = runner.run(
//        &proptest::collection::vec(any::<TestingAction>(), 0..50),
//        |actions| {
//            let tokio_runtime = tokio::runtime::Builder::new_multi_thread()
//                .enable_all()
//                .build()
//                .unwrap();
//            tokio_runtime.block_on(async {
//                preferred_sequencer_is_resistant_to_miscellaneous_edge_cases(actions).await;
//            });
//
//            Ok(())
//        },
//    );
//
//    result.unwrap();
//}

#[tokio::test(flavor = "multi_thread")]
async fn batch_production_and_accept_tx() {
    let mut actions = vec![];
    for i in 1..20 {
        actions.push(TestingAction::AcceptTxs { count: i });
        actions.push(TestingAction::AcceptTx);
    }

    preferred_sequencer_is_resistant_to_miscellaneous_edge_cases(actions).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn restart_and_query_value() {
    let actions = vec![TestingAction::Restart, TestingAction::QuerySetValue];

    preferred_sequencer_is_resistant_to_miscellaneous_edge_cases(actions).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn batch_production_and_state_query() {
    let actions = vec![
        TestingAction::AcceptTx,
        TestingAction::AcceptTxs { count: 4 },
        TestingAction::AcceptTxs { count: 4 },
        TestingAction::AcceptTxs { count: 4 },
        TestingAction::AcceptTxs { count: 4 },
        TestingAction::AcceptTxs { count: 4 },
        TestingAction::AcceptTxs { count: 4 },
        TestingAction::QuerySetValue,
        TestingAction::AcceptTxs { count: 4 },
        TestingAction::AcceptTx,
        TestingAction::QuerySetValue,
    ];

    preferred_sequencer_is_resistant_to_miscellaneous_edge_cases(actions).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn api_state_race_condition_regression() {
    let actions = vec![
        TestingAction::QuerySetValue,
        TestingAction::AcceptTx,
        TestingAction::QuerySetValue,
        TestingAction::AcceptTxs { count: 1 },
        TestingAction::QuerySetValue,
        TestingAction::AcceptTx,
        TestingAction::AcceptTx,
        TestingAction::AcceptTx,
        TestingAction::TryAcceptBadTx {
            invalid_reason: InvalidGeneration::DuplicateTransaction,
        },
        TestingAction::TryAcceptBadTx {
            invalid_reason: InvalidGeneration::TooOld,
        },
        TestingAction::AcceptTx,
        TestingAction::AcceptTx,
        TestingAction::QuerySetValue,
        TestingAction::TryAcceptBadTx {
            invalid_reason: InvalidGeneration::TooOld,
        },
        TestingAction::NewDaSlot {},
        TestingAction::QuerySetValue,
        TestingAction::QuerySetValue,
        TestingAction::QuerySetValue,
        TestingAction::QuerySetValue,
        TestingAction::AcceptTxs { count: 4 },
        TestingAction::TryAcceptBadTx {
            invalid_reason: InvalidGeneration::TooOld,
        },
        TestingAction::AcceptTxs { count: 4 },
        TestingAction::QuerySetValue,
    ];

    preferred_sequencer_is_resistant_to_miscellaneous_edge_cases(actions).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn restart_after_big_batch_regression() {
    let actions = vec![
        TestingAction::AcceptTxs { count: 1 },
        TestingAction::AcceptTxs { count: 5 },
        TestingAction::AcceptTx,
        TestingAction::Restart,
        TestingAction::AcceptTxs { count: 10 },
        TestingAction::Restart,
        TestingAction::AcceptTx,
    ];

    preferred_sequencer_is_resistant_to_miscellaneous_edge_cases(actions).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn batch_production_with_immediate_finalization() {
    let actions = vec![
        TestingAction::AcceptTxs { count: 1 },
        TestingAction::AcceptTxs { count: 50 },
        TestingAction::Restart,
        TestingAction::AcceptTx,
        TestingAction::Sleep { duration_ms: 50 },
        TestingAction::Restart,
        TestingAction::AcceptTxs { count: 50 },
        TestingAction::Restart,
        TestingAction::AcceptTx,
        TestingAction::AcceptTx,
        TestingAction::AcceptTx,
        TestingAction::AcceptTx,
        TestingAction::AcceptTx,
        TestingAction::AcceptTx,
        TestingAction::AcceptTxs { count: 3 },
        TestingAction::AcceptTxs { count: 50 },
    ];

    preferred_sequencer_is_resistant_to_miscellaneous_edge_cases(actions).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_sequencer_state_and_node_state_matches() {
    let actions = vec![
        TestingAction::PauseUpdateStateExecution(true),
        TestingAction::AcceptTxs { count: 50 },
        // Query the state the sequencer has produced
        // Sequencer state updates are currently paused so we dont
        // update with state from the node.
        TestingAction::QuerySetValue,
        TestingAction::PauseUpdateStateExecution(false),
        // Produce a block to trigger an update_state operation
        // which will replace the state with that from the node.
        // It should produce the same result as our previous query
        // This should produce a batch containing previous transactions
        TestingAction::NewDaSlotAndWait,
        // Trigger another block so we get updated state
        TestingAction::NewDaSlotAndWait,
        // Should be unchanged
        TestingAction::QuerySetValue,
    ];

    preferred_sequencer_is_resistant_to_miscellaneous_edge_cases(actions).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn api_state_race_condition_regression2() {
    let actions = vec![
        TestingAction::QuerySetValue,
        TestingAction::AcceptTx,
        TestingAction::QuerySetValue,
        TestingAction::AcceptTxs { count: 1 },
        TestingAction::QuerySetValue,
        TestingAction::AcceptTx,
        TestingAction::AcceptTx,
        TestingAction::AcceptTx,
        TestingAction::TryAcceptBadTx {
            invalid_reason: InvalidGeneration::DuplicateTransaction,
        },
        TestingAction::TryAcceptBadTx {
            invalid_reason: InvalidGeneration::TooOld,
        },
        TestingAction::AcceptTx,
        TestingAction::AcceptTx,
        TestingAction::QuerySetValue,
        TestingAction::TryAcceptBadTx {
            invalid_reason: InvalidGeneration::TooOld,
        },
        TestingAction::NewDaSlot {},
        TestingAction::QuerySetValue,
        TestingAction::QuerySetValue,
        TestingAction::QuerySetValue,
        TestingAction::QuerySetValue,
        TestingAction::AcceptTxs { count: 4 },
        TestingAction::TryAcceptBadTx {
            invalid_reason: InvalidGeneration::TooOld,
        },
        TestingAction::AcceptTxs { count: 4 },
        TestingAction::QuerySetValue,
    ];

    preferred_sequencer_is_resistant_to_miscellaneous_edge_cases(actions).await;
}
