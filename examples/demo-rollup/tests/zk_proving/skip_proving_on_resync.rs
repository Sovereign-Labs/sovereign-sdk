use futures::StreamExt;
use sov_api_spec::types::AggregatedProof as ApiAggregatedProof;
use sov_mock_zkvm::{MockCodeCommitment, MockZkVerifier};
use sov_modules_api::{SerializedAggregatedProof, Spec};
use sov_rollup_interface::zk::aggregated_proof::{
    AggregateProofVerifier, AggregatedProofPublicData,
};
use sov_state::Storage;
use sov_test_utils::TEST_DEFAULT_MOCK_DA_PERIODIC_PRODUCING;

use crate::external_mock_da::start_external_mock_da;
use crate::test_helpers::DemoRollupSpec;
use crate::zk_proving::start_test_rollup;

#[tokio::test(flavor = "multi_thread")]
async fn skip_proving_on_resync() -> anyhow::Result<()> {
    // sov_test_utils::initialize_logging();
    let external_da = start_external_mock_da(TEST_DEFAULT_MOCK_DA_PERIODIC_PRODUCING).await?;
    let test_rollup = start_test_rollup(3, &external_da, 20).await?;
    let mut proof_sub = test_rollup.subscribe_aggregated_proof().await?;

    for _ in 0..5 {
        let _ = proof_sub.next().await.unwrap().unwrap();
    }

    let h = test_rollup.height().await;
    println!("Start Height {}", h);
    let builder = test_rollup.shutdown().await?;

    let mut header_subscription = external_da.service.subscribe_finalized_header().await?;
    for i in 0..10 {
        let header = header_subscription.next().await.unwrap().unwrap();
        println!("Header {}", header.height)
    }

    println!("XXXXXXXXX");

    let test_rollup = builder.start_test_rollup().await?;
    let h = test_rollup.height().await;
    println!("Start Height {}", h);

    let mut proof_sub = test_rollup.subscribe_aggregated_proof().await?;

    for _ in 0..5 {
        println!("WITING FOR PROOF");
        let _ = proof_sub.next().await.unwrap().unwrap();
    }

    //test_rollup.wait_for_rollup_height_advance_by(15).await;
    let h = test_rollup.height().await;
    println!("End Height {}", h);

    Ok(())
}
