use std::time::Duration;

use anyhow::Context;
use futures::StreamExt;

use crate::zk_proving::start_test_rollup;

const AGGREGATION_GATE_ENV: &str = "SOV_MOCK_AGGREGATION_GATE";

fn stop_proving() {
    std::env::set_var(AGGREGATION_GATE_ENV, "STOP_PROVING");
}

fn resume_proving() {
    std::env::remove_var(AGGREGATION_GATE_ENV);
}

#[tokio::test(flavor = "multi_thread")]
async fn max_concurrent_proof() -> anyhow::Result<()> {
    let test_rollup = start_test_rollup(0).await?;

    let mut aggregated_proof_subscription = test_rollup
        .client
        .client
        .subscribe_aggregated_proof()
        .await
        .context("Failed to subscribe to aggregated proof")?;

    let _ = aggregated_proof_subscription.next().await.unwrap().unwrap();

    stop_proving();
    let mut finalized_slots = test_rollup
        .client
        .client
        .subscribe_finalized_slots()
        .await
        .context("Failed to subscribe to aggregated proof")?;

    for _ in 0..20 {
        let _ = finalized_slots.next().await.unwrap()?;
    }
    resume_proving();
    test_rollup
        .wait_for_rollup_to_shutdown(Duration::from_secs(10))
        .await;

    Ok(())
}
