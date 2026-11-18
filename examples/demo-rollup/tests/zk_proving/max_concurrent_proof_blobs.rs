use crate::test_helpers::NOTIFICATION_TIMEOUT;
use sov_test_utils::test_rollup::next_within;
use std::time::Duration;

use sov_test_utils::TEST_DEFAULT_MOCK_DA_PERIODIC_PRODUCING;

use crate::external_mock_da::start_external_mock_da;
use crate::zk_proving::{query_verified_proofs, start_test_rollup};

const AGGREGATION_GATE_ENV: &str = "SOV_MOCK_AGGREGATION_GATE";

fn stop_proving() {
    std::env::set_var(AGGREGATION_GATE_ENV, "STOP_PROVING");
}

fn resume_proving() {
    std::env::remove_var(AGGREGATION_GATE_ENV);
}

// Flaky: depends on timing. The aggregator shuts the node down when a status check sees
// `max_concurrent_proof_blobs` proof blobs that are not finalized yet. The test needs that to
// happen only after `resume_proving`, but two things can trigger it earlier or prevent it:
// - If the node falls behind the DA at startup, catching up yields a burst of proofs that
//   reaches the limit before the first proof is observed.
// - The in-flight counter rises only when a blob submission starts, so after the post-resume
//   burst the check can still read a low value.
// The limit is 3 rather than 2 because a proof stays in flight until finalized (~3 blocks) while
// a new one is sent every 2 blocks, so 2 are routinely in flight even in normal operation.
#[tokio::test(flavor = "multi_thread")]
async fn flaky_max_concurrent_proof() -> anyhow::Result<()> {
    let external_da = start_external_mock_da(TEST_DEFAULT_MOCK_DA_PERIODIC_PRODUCING).await?;
    let test_rollup = start_test_rollup(0, &external_da, 3, false, None).await?;

    let mut aggregated_proof_subscription = test_rollup.subscribe_aggregated_proof().await?;
    let _ = next_within(&mut aggregated_proof_subscription, NOTIFICATION_TIMEOUT)
        .await
        .unwrap();

    stop_proving();
    let mut finalized_slots = test_rollup.subscribe_finalized_slots().await?;

    for _ in 0..20 {
        let _ = next_within(&mut finalized_slots, NOTIFICATION_TIMEOUT).await?;
    }
    resume_proving();

    let agg_pub_data = query_verified_proofs(&test_rollup).await?;
    assert!(agg_pub_data.final_slot_number.get() > 1);

    test_rollup
        .wait_for_rollup_to_shutdown(Duration::from_secs(10))
        .await;

    Ok(())
}
