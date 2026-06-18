//! Off-circuit, host-side light-client support, shared across zkVMs.
//!
//! A light client verifies a rollup's latest aggregated ("outer") proof without
//! running a full node: it fetches the proof from a running node's REST API,
//! cryptographically verifies it, and returns the committed public values. The
//! fetch step is identical for every zkVM, so it lives here; each adapter only
//! provides its own [`ZkLightClient::verify_aggregated_proof`] implementation.
//!
//! This module is only available under the `native` feature, since fetching and
//! verifying a proof is a host-side concern.

use std::time::Duration;

use anyhow::Context as _;
use base64::Engine as _;
use serde::de::DeserializeOwned;

use crate::da::DaSpec;
use crate::zk::aggregated_proof::{AggregatedProofPublicData, SerializedAggregatedProof};

/// Total timeout for a single light-client request, including connecting,
/// sending the request, and reading the full response body.
const HTTP_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Timeout for establishing the TCP connection to the node.
const HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Verifies a rollup's latest aggregated ("outer") proof without running a full
/// node.
///
/// A light client stores no state and replays no blocks. It only fetches the
/// latest aggregated proof from a running node, derives the outer verification
/// key for its zkVM, cryptographically verifies the proof, and returns its
/// public values.
///
/// Implementations are provided per zkVM by the respective adapter crates (e.g.
/// `sov_sp1_adapter::light_client::Sp1LightClient`,
/// `sov_mock_zkvm::light_client::MockLightClient`).
#[async_trait::async_trait]
pub trait ZkLightClient {
    /// Verifies an already-fetched serialized aggregated proof against this
    /// client's trusted verification keys and returns its public values.
    fn verify_aggregated_proof<Address, Da, Root>(
        &self,
        proof: SerializedAggregatedProof,
    ) -> anyhow::Result<AggregatedProofPublicData<Address, Da, Root>>
    where
        Address: DeserializeOwned,
        Da: DaSpec,
        Root: DeserializeOwned;

    /// Returns the [`reqwest::Client`] used to fetch proofs from the node.
    fn http_client(&self) -> &reqwest::Client;

    /// Builds the [`reqwest::Client`] every [`ZkLightClient`] uses to fetch
    /// proofs, preconfigured with the light client's default request and
    /// connection timeouts.
    fn build_http_client() -> reqwest::Client {
        reqwest::Client::builder()
            .timeout(HTTP_REQUEST_TIMEOUT)
            .connect_timeout(HTTP_CONNECT_TIMEOUT)
            .build()
            .expect("Failed to build the light-client HTTP client")
    }

    /// Fetches the latest aggregated proof from the node at `url`.
    async fn fetch_and_verify_latest_aggregated_proof<Address, Da, Root>(
        &self,
        url: &str,
    ) -> anyhow::Result<AggregatedProofPublicData<Address, Da, Root>>
    where
        Address: DeserializeOwned,
        Da: DaSpec,
        Root: DeserializeOwned,
    {
        let proof = fetch_latest_aggregated_proof(self.http_client(), url).await?;
        self.verify_aggregated_proof(proof)
    }
}

#[derive(serde::Deserialize)]
struct AggregatedProofResponse {
    proof: String,
}

async fn fetch_latest_aggregated_proof(
    client: &reqwest::Client,
    url: &str,
) -> anyhow::Result<SerializedAggregatedProof> {
    let response: AggregatedProofResponse = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("Failed to request the latest aggregated proof from {url}"))?
        .error_for_status()
        .context("Node returned an error for the latest aggregated proof")?
        .json()
        .await
        .context("Failed to decode the aggregated proof response")?;

    let raw_aggregated_proof = base64::engine::general_purpose::STANDARD
        .decode(response.proof)
        .context("Failed to base64-decode the aggregated proof")?;

    Ok(SerializedAggregatedProof {
        raw_aggregated_proof,
    })
}
