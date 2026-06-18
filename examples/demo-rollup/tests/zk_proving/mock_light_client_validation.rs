//! End-to-end test: a running rollup produces aggregated ("outer") proofs, and
//! a [`MockLightClient`] independently fetches and verifies the latest one from
//! the node's REST API — exactly as the real light-client binary would.

use futures::StreamExt;
use sov_demo_rollup::read_mock_code_commitments_from_env;
use sov_mock_zkvm::light_client::MockLightClient;
use sov_modules_api::{CodeCommitmentTrait, Spec};
use sov_node_client::NodeClient;
use sov_rollup_interface::zk::ZkLightClient;
use sov_state::Storage;
use sov_test_utils::TEST_DEFAULT_MOCK_DA_PERIODIC_PRODUCING;

use crate::external_mock_da::start_external_mock_da;
use crate::zk_proving::{start_test_rollup, ProofPublicData, RollupSpec};

type Address = <RollupSpec as Spec>::Address;
type Da = <RollupSpec as Spec>::Da;
type Root = <<RollupSpec as Spec>::Storage as Storage>::Root;

/// The rollup creates aggregated proofs while running; a `MockLightClient`
/// fetches the latest one from the node's REST endpoint and verifies it against
/// the outer code commitment, proving the proof is valid without trusting the node.
#[tokio::test(flavor = "multi_thread")]
async fn light_client_validates_aggregated_proof() -> anyhow::Result<()> {
    let external_da = start_external_mock_da(TEST_DEFAULT_MOCK_DA_PERIODIC_PRODUCING).await?;
    let test_rollup = start_test_rollup(0, &external_da, 20, false, None).await?;

    // 1. Rollup creates some aggregated proofs. Wait until at least one has been
    //    produced and served by the node.
    let mut aggregated_proof_subscription = test_rollup.subscribe_aggregated_proof().await?;
    let _ = aggregated_proof_subscription.next().await.unwrap()?;

    // 2. Validate the latest aggregated proof with a MockLightClient. The
    //    NodeClient talks to the node's `/ledger/aggregated-proofs/latest`
    //    endpoint and the light client verifies the proof against the inner and
    //    outer code commitments the node was configured with.
    let node_url = format!("http://{}", test_rollup.http_addr);
    let (inner_code_commitment, outer_code_commitment) = read_mock_code_commitments_from_env();

    let light_client = MockLightClient::new(inner_code_commitment.to_hash(), outer_code_commitment);
    let proof = NodeClient::new_unchecked(&node_url)
        .fetch_latest_aggregated_proof()
        .await?;
    let public_data: ProofPublicData =
        light_client.verify_aggregated_proof::<Address, Da, Root>(proof)?;

    // 3. The verified public data describes a non-empty slot range.
    assert!(
        public_data.final_slot_number.get() >= public_data.initial_slot_number.get(),
        "aggregated proof final slot should be >= initial slot"
    );

    let _ = test_rollup.shutdown().await;

    Ok(())
}
