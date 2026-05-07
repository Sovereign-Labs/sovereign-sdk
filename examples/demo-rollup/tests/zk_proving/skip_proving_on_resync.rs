use sov_test_utils::TEST_DEFAULT_MOCK_DA_PERIODIC_PRODUCING;

use crate::external_mock_da::start_external_mock_da;
use crate::zk_proving::start_test_rollup;

#[tokio::test(flavor = "multi_thread")]
async fn skip_proving_on_resync() -> anyhow::Result<()> {
    let external_da = start_external_mock_da(TEST_DEFAULT_MOCK_DA_PERIODIC_PRODUCING).await?;
    let test_rollup = start_test_rollup(3, &external_da).await?;
    test_rollup.wait_for_rollup_height_advance_by(10);

    let _ = test_rollup.shutdown().await;

    let header_subscription = external_da.service.subscribe_finalized_header().await?;
    for i in 0..10 {
        let header = header_subscription.next().await.unwrap().unwrap().await;
        println!("Header {}", header.height)
    }

    let test_rollup = start_test_rollup(3, &external_da).await?;

    Ok(())
}
