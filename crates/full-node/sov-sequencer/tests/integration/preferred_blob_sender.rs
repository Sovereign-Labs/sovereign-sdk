use std::{env, fs, time::Duration};

use futures::StreamExt;
use sov_blob_sender::{BlobSelectorStatus, BlobSubmissionStatus};
use sov_mock_da::BlockProducingConfig;
use sov_modules_api::prelude::*;
use sov_rollup_interface::stf::BlobDiscardReason;
use sov_test_utils::test_rollup::{next_within, TestRollup};
use sov_test_utils::{TestSpec, TestUser};

#[allow(unused_imports)]
use crate::preferred_end_to_end::{
    create_test_rollup_with, run_action_against_test_rollup, run_actions_against_test_rollup,
    set_many_values, setup_test_rollup_with_initial_state, InvalidGeneration, TestBlueprint,
    TestRuntime, TestState, TestingAction,
};

const PREFERRED_SEQUENCER_DB_NAME: &str = "preferred_sequencer";

async fn create_test_rollup() -> (TestRollup<TestBlueprint>, TestUser<TestSpec>) {
    create_test_rollup_with(|b| {
        b.block_producing_config(BlockProducingConfig::Periodic { block_time_ms: 300 })
            .finalization_blocks(0)
    })
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn test_startup_fails_if_blob_sender_db_is_ahead_of_preferred_db() {
    sov_test_utils::initialize_logging();
    let (test_rollup, admin) = create_test_rollup().await;

    test_rollup.produce_enough_finalized_slots().await;
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    test_rollup
        .submit(&admin, set_many_values(vec![7; 100]))
        .await
        .unwrap();

    let mut blob_sender_statuses = test_rollup
        .subscribe_to_blobs_from_blob_sender()
        .await
        .unwrap();

    // Pause DA submission after BlobSender persists the blob locally, before it can land on DA.
    test_rollup.da_service.set_blob_submission_pause().await;
    test_rollup.force_close_batch().await.unwrap();
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let blob_status = blob_sender_statuses
                .next()
                .await
                .expect("BlobSender status stream closed before the blob was persisted")
                .expect("BlobSender status stream returned an error");
            if matches!(
                blob_status.blob_submission_status,
                BlobSubmissionStatus::MustSubmit
            ) {
                break;
            }
        }
    })
    .await
    .expect("Timeout occurred while waiting for the blob to be persisted in BlobSender DB.");

    let builder = test_rollup.shutdown().await.unwrap();
    let storage_path = builder.storage_path();
    let preferred_db_path = storage_path.path().join(PREFERRED_SEQUENCER_DB_NAME);
    assert!(
        preferred_db_path.exists(),
        "Preferred sequencer DB should exist before simulating a wipe"
    );
    // Simulate wiping the preferred sequencer DB while leaving BlobSender DB intact.
    fs::remove_dir_all(preferred_db_path).unwrap();

    let err = match builder.start().await {
        Ok(test_rollup) => {
            test_rollup.shutdown().await.unwrap();
            panic!("Rollup started despite BlobSender DB being ahead of preferred sequencer DB");
        }
        Err(err) => err,
    };

    let err = err.to_string();
    assert!(
        err.contains("Node state is inconsistent, aborting startup: BlobSender DB contains a higher blob sequence number than the Preferred Sequencer DB."),
        "Unexpected startup error: {err}"
    );
    assert!(
        err.contains("preferred sequencer only has blobs up to none"),
        "Unexpected startup error: {err}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_discard_oversized_blobs() {
    env::set_var(
        "SOV_TEST_CONST_OVERRIDE_MAX_ALLOWED_DATA_SIZE_RETURNED_BY_BLOB_STORAGE",
        "1000",
    );
    let (test_rollup, admin) = create_test_rollup().await;

    test_rollup.produce_enough_finalized_slots().await;
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    // Blob with this transaction will be discarded becuse the blob is bigger than `MAX_ALLOWED_DATA_SIZE_RETURNED_BY_BLOB_STORAGE`
    test_rollup
        .submit(&admin, set_many_values(vec![7; 10000]))
        .await
        .unwrap();
    let mut sub = test_rollup
        .subscribe_to_blobs_from_blob_sender()
        .await
        .unwrap();

    tokio::time::timeout(tokio::time::Duration::from_secs(15), async {
        while let Some(blob_status) = sub.next().await {
            if let Some(BlobSelectorStatus::Discarded(BlobDiscardReason::OutOfCapacity)) =
                blob_status.as_ref().unwrap().blob_selector_status
            {
                break;
            }
        }
    })
    .await
    .expect("Timeout occurred while waiting for the discarded blob.");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_blobs_are_send_after_rollup_resync() {
    sov_test_utils::initialize_logging();
    let (test_rollup, _) = create_test_rollup().await;
    let da = test_rollup.da_service.clone();
    let mut header_subscription = da.subscribe_finalized_header().await.unwrap();

    for _ in 0..10 {
        da.produce_block_now().await.unwrap();
        next_within(
            &mut header_subscription,
            TestRollup::<TestBlueprint>::POLLING_TIMEOUT,
        )
        .await
        .unwrap();
    }
    test_rollup.wait_for_node_synced().await.unwrap();

    let builder = test_rollup.shutdown().await.unwrap();

    // Generate a block while Rollup is offline to trigger resync logic.
    for _ in 0..20 {
        da.produce_block_now().await.unwrap();
        next_within(
            &mut header_subscription,
            TestRollup::<TestBlueprint>::POLLING_TIMEOUT,
        )
        .await
        .unwrap();
    }

    // The new rollup has pending blobs in the BlobSender DB and completed blobs in the Preferred Sequencer state.
    let test_rollup = builder.start().await.unwrap();
    let mut subscribe_state_updates = test_rollup.subscribe_state_updates().await.unwrap();
    let mut subscribe_to_blobs_from_blob_sender = test_rollup
        .subscribe_to_blobs_from_blob_sender()
        .await
        .unwrap();

    // BlobSender should send blobs only after resync is complete, so the subscribe_state_updates notification must come first.
    tokio::select! {
        _ = subscribe_state_updates.next() => {}
        _ = subscribe_to_blobs_from_blob_sender.next() => {
            panic!("In a resync scenario, the state update notification should occur before the blob sender transmits the blobs.")
        }
    }
}
