use futures::StreamExt;
use sov_demo_rollup::ExternalMockDemoRollup;
use sov_modules_api::execution_mode::Native;
use sov_rollup_interface::common::SlotNumber;
use sov_test_utils::test_rollup::TestRollup;
use sov_test_utils::TEST_DEFAULT_MOCK_DA_PERIODIC_PRODUCING;

use crate::external_mock_da::start_external_mock_da;
use crate::zk_proving::{query_verified_proofs, start_test_rollup};

#[tokio::test(flavor = "multi_thread")]
async fn skip_proving_on_resync() -> anyhow::Result<()> {
    let external_da = start_external_mock_da(TEST_DEFAULT_MOCK_DA_PERIODIC_PRODUCING).await?;
    let test_rollup = start_test_rollup(3, &external_da, 20, false).await?;
    let mut proof_sub = test_rollup.subscribe_aggregated_proof().await?;

    for _ in 0..5 {
        let _ = proof_sub.next().await.unwrap().unwrap();
    }

    let agg_pub_data = query_verified_proofs(&test_rollup).await?;
    assert_eq!(agg_pub_data.origin_slot_number, SlotNumber::GENESIS);

    let test_rollup = restart_with_start_fresh_outer_proof_on_resync(test_rollup).await?;
    test_rollup.wait_for_rollup_height_advance_by(20).await;

    let agg_pub_data = query_verified_proofs(&test_rollup).await?;
    assert!(agg_pub_data.origin_slot_number > SlotNumber::GENESIS);

    Ok(())
}

async fn restart_with_start_fresh_outer_proof_on_resync(
    test_rollup: TestRollup<ExternalMockDemoRollup<Native>>,
) -> anyhow::Result<TestRollup<ExternalMockDemoRollup<Native>>> {
    let builder = test_rollup.shutdown().await?;
    let builder = builder.set_start_fresh_outer_proof_on_resync(true);
    builder.start_test_rollup().await
}
