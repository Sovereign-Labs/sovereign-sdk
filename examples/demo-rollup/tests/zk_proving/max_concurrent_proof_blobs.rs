use std::time::Duration;

use futures::StreamExt;
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

#[tokio::test(flavor = "multi_thread")]
async fn max_concurrent_proof() -> anyhow::Result<()> {
    let external_da = start_external_mock_da(TEST_DEFAULT_MOCK_DA_PERIODIC_PRODUCING).await?;
    let test_rollup = start_test_rollup(0, &external_da, 2).await?;

    let mut aggregated_proof_subscription = test_rollup.subscribe_aggregated_proof().await?;
    let _ = aggregated_proof_subscription.next().await.unwrap().unwrap();

    stop_proving();
    let mut finalized_slots = test_rollup.subscribe_finalized_slots().await?;

    for _ in 0..20 {
        let _ = finalized_slots.next().await.unwrap()?;
    }
    resume_proving();

    let agg_pub_data = query_verified_proofs(&test_rollup).await?;
    assert!(agg_pub_data.final_slot_number.get() > 1);

    test_rollup
        .wait_for_rollup_to_shutdown(Duration::from_secs(10))
        .await;

    Ok(())
}
