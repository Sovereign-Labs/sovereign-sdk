//! Mock-zkVM light-client verification for the demo rollup.
//!
//! Reads the inner/outer code commitments from the environment, then fetches and
//! verifies the node's latest aggregated proof against them.

use sov_demo_rollup::{read_mock_code_commitments_from_env, MockRollupSpec};
use sov_mock_zkvm::light_client::MockLightClient;
use sov_modules_api::execution_mode::Native;
use sov_modules_api::{CodeCommitmentTrait, Spec, Storage};
use sov_node_client::NodeClient;
use sov_rollup_interface::zk::ZkLightClient;

/// Public-value types committed to by the mock-zkVM demo rollup, mirroring the
/// spec the node runs with ([`MockDemoRollup`](sov_demo_rollup::MockDemoRollup)).
type Address = <MockRollupSpec<Native> as Spec>::Address;
type Da = <MockRollupSpec<Native> as Spec>::Da;
type Root = <<MockRollupSpec<Native> as Spec>::Storage as Storage>::Root;

/// Fetches the node's latest aggregated proof from the node at `node_url`,
/// verifies it with a [`MockLightClient`] built from the environment-configured
/// code commitments, and prints the verified public values.
pub async fn verify_latest_aggregated_proof(node_url: &str) -> anyhow::Result<()> {
    let (inner_code_commitment, outer_code_commitment) = read_mock_code_commitments_from_env();

    let light_client = MockLightClient::new(inner_code_commitment.to_hash(), outer_code_commitment);
    let proof = NodeClient::new_unchecked(node_url)
        .fetch_latest_aggregated_proof()
        .await?;
    let public_data = light_client.verify_aggregated_proof::<Address, Da, Root>(proof)?;
    println!("Proof verified successfully. Public values:\n{public_data:#?}");

    Ok(())
}
