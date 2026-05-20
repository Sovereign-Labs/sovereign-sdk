use crate::external_mock_da::start_external_mock_da;
use crate::zk_proving::start_test_rollup;
use anyhow::Context;
use futures::StreamExt;
use sov_demo_rollup::{set_inner_code_commitment_env, set_outer_code_commitment_env};
use sov_mock_zkvm::MockCodeCommitment;
use sov_modules_api::capabilities::RollupHeight;
use sov_test_utils::TEST_DEFAULT_MOCK_DA_PERIODIC_PRODUCING;
use std::time::Duration;

const STOP_AT: u64 = 20;

#[tokio::test(flavor = "multi_thread")]
async fn code_commitment_rotation_requires_fresh_start() -> anyhow::Result<()> {
    let external_da = start_external_mock_da(TEST_DEFAULT_MOCK_DA_PERIODIC_PRODUCING).await?;

    // Phase 1 — env vars unset, defaults everywhere. Runner self-shuts at STOP_AT.
    let test_rollup =
        start_test_rollup(3, &external_da, 20, false, Some(RollupHeight::new(STOP_AT))).await?;

    let builder = test_rollup
        .wait_for_rollup_to_shutdown(Duration::from_secs(60))
        .await;

    // Phase 2 — rotate commitments and restart on the same storage.
    set_inner_code_commitment_env(MockCodeCommitment([0x01; 8]));
    set_outer_code_commitment_env(MockCodeCommitment([0x02; 8]));

    // This is an error because the persisted aggregated proof was stamped with the
    // default code commitments, and the rotated env vars no longer match its
    // inner/outer vkey hashes — startup refuses to resume on top of a proof it can't
    // verify against the current commitments.

    let err = match builder
        .clone()
        .set_config(|c| {
            c.start_at_rollup_height = Some(RollupHeight::new(STOP_AT + 1));
            c.stop_at_rollup_height = None;
        })
        .start_test_rollup()
        .await
    {
        Ok(_) => anyhow::bail!("expected resume to fail under rotated commitments"),
        Err(err) => err,
    };

    assert!(
        format!("{err:#}").contains("code commitment changed since last proof"),
        "expected commitment mismatch error, got: {err:#}"
    );

    // With start_fresh_outer_proof_on_resync = true, startup drops the previous
    // outer-proof and begins a fresh aggregation chain under the rotated
    // commitments — no vkey-hash check, so this restart succeeds.
    let test_rollup = builder
        .set_config(|c| {
            c.start_at_rollup_height = Some(RollupHeight::new(STOP_AT + 1));
            c.stop_at_rollup_height = None;
            c.start_fresh_outer_proof_on_resync = true;
        })
        .start_test_rollup()
        .await?;

    let mut proof_sub = test_rollup.subscribe_aggregated_proof().await.unwrap();

    for i in 0..5 {
        tokio::time::timeout(Duration::from_secs(20), proof_sub.next())
            .await
            .with_context(|| format!("no aggregated proof within 60s on iter={i}"))?
            .context("proof error")??;
    }

    Ok(())
}
