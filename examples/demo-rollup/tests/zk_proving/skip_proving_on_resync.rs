use crate::zk_proving::start_test_rollup;
/*

#[tokio::test(flavor = "multi_thread")]
async fn skip_proving_on_resync() -> anyhow::Result<()> {
    let (test_rollup, _external_da) = start_test_rollup(3).await?;

    let mut finalized_slots = test_rollup
        .client
        .client
        .subscribe_finalized_slots()
        .await
        .context("Failed to subscribe to aggregated proof")?;

    Ok(())
}*/
