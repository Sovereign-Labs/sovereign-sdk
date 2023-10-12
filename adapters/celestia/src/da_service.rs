use async_trait::async_trait;
use celestia_rpc::prelude::*;
use celestia_types::blob::{Blob as JsonBlob, Commitment, SubmitOptions};
use celestia_types::consts::appconsts::{
    CONTINUATION_SPARSE_SHARE_CONTENT_SIZE, FIRST_SPARSE_SHARE_CONTENT_SIZE, SHARE_SIZE,
};
use celestia_types::nmt::Namespace;
use jsonrpsee::http_client::{HeaderMap, HttpClient};
use sov_rollup_interface::da::CountedBufReader;
use sov_rollup_interface::services::da::DaService;
use tracing::{debug, info, instrument, trace};

use crate::shares::Blob;
use crate::types::FilteredCelestiaBlock;
use crate::utils::BoxError;
use crate::verifier::proofs::{CompletenessProof, CorrectnessProof};
use crate::verifier::{CelestiaSpec, CelestiaVerifier, RollupParams, PFB_NAMESPACE};
use crate::BlobWithSender;

// Approximate value, just to make it work.
const GAS_PER_BYTE: usize = 20;
const GAS_PRICE: usize = 1;

#[derive(Debug, Clone)]
pub struct CelestiaService {
    client: HttpClient,
    rollup_namespace: Namespace,
}

impl CelestiaService {
    pub fn with_client(client: HttpClient, nid: Namespace) -> Self {
        Self {
            client,
            rollup_namespace: nid,
        }
    }
}

/// Runtime configuration for the DA service
#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct DaServiceConfig {
    /// The jwt used to authenticate with the Celestia rpc server
    pub celestia_rpc_auth_token: String,
    /// The address of the Celestia rpc server
    #[serde(default = "default_rpc_addr")]
    pub celestia_rpc_address: String,
    /// The maximum size of a Celestia RPC response, in bytes
    #[serde(default = "default_max_response_size")]
    pub max_celestia_response_body_size: u32,
    /// The timeout for a Celestia RPC request, in seconds
    #[serde(default = "default_request_timeout_seconds")]
    pub celestia_rpc_timeout_seconds: u64,
}

fn default_rpc_addr() -> String {
    "http://localhost:11111/".into()
}

fn default_max_response_size() -> u32 {
    1024 * 1024 * 100 // 100 MB
}

const fn default_request_timeout_seconds() -> u64 {
    60
}

impl CelestiaService {
    pub async fn new(config: DaServiceConfig, chain_params: RollupParams) -> Self {
        let client = {
            let mut headers = HeaderMap::new();
            headers.insert(
                "Authorization",
                format!("Bearer {}", config.celestia_rpc_auth_token)
                    .parse()
                    .unwrap(),
            );

            jsonrpsee::http_client::HttpClientBuilder::default()
                .set_headers(headers)
                .max_request_size(config.max_celestia_response_body_size)
                .request_timeout(std::time::Duration::from_secs(
                    config.celestia_rpc_timeout_seconds,
                ))
                .build(&config.celestia_rpc_address)
        }
        .expect("Client initialization is valid");

        Self::with_client(client, chain_params.namespace)
    }
}

#[async_trait]
impl DaService for CelestiaService {
    type Spec = CelestiaSpec;

    type Verifier = CelestiaVerifier;

    type FilteredBlock = FilteredCelestiaBlock;

    type Error = BoxError;

    #[instrument(skip(self), err)]
    async fn get_finalized_at(&self, height: u64) -> Result<Self::FilteredBlock, Self::Error> {
        let client = self.client.clone();
        let rollup_namespace = self.rollup_namespace;

        // Fetch the header and relevant shares via RPC
        debug!("Fetching header");
        let header = client.header_get_by_height(height).await?;
        trace!(header_result = ?header);

        // Fetch the rollup namespace shares, etx data and extended data square
        debug!("Fetching rollup data...");
        let rollup_rows_future = client.share_get_shares_by_namespace(&header, rollup_namespace);
        let etx_rows_future = client.share_get_shares_by_namespace(&header, PFB_NAMESPACE);
        let data_square_future = client.share_get_eds(&header);

        let (rollup_rows, etx_rows, data_square) =
            tokio::try_join!(rollup_rows_future, etx_rows_future, data_square_future)?;

        FilteredCelestiaBlock::new(
            self.rollup_namespace,
            header,
            rollup_rows,
            etx_rows,
            data_square,
        )
    }

    async fn get_block_at(&self, height: u64) -> Result<Self::FilteredBlock, Self::Error> {
        self.get_finalized_at(height).await
    }

    fn extract_relevant_blobs(
        &self,
        block: &Self::FilteredBlock,
    ) -> Vec<<Self::Spec as sov_rollup_interface::da::DaSpec>::BlobTransaction> {
        let mut output = Vec::new();
        for blob_ref in block.rollup_data.blobs() {
            let commitment = Commitment::from_shares(self.rollup_namespace, blob_ref.0)
                .expect("blob must be valid");
            info!("Blob: {:?}", commitment);
            let sender = block
                .relevant_pfbs
                .get(&commitment.0[..])
                .expect("blob must be relevant")
                .0
                .signer
                .clone();

            let blob: Blob = blob_ref.into();

            let blob_tx = BlobWithSender {
                blob: CountedBufReader::new(blob.into_iter()),
                sender: sender.parse().expect("Incorrect sender address"),
                hash: commitment.0,
            };

            output.push(blob_tx)
        }
        output
    }

    async fn get_extraction_proof(
        &self,
        block: &Self::FilteredBlock,
        blobs: &[<Self::Spec as sov_rollup_interface::da::DaSpec>::BlobTransaction],
    ) -> (
        <Self::Spec as sov_rollup_interface::da::DaSpec>::InclusionMultiProof,
        <Self::Spec as sov_rollup_interface::da::DaSpec>::CompletenessProof,
    ) {
        let etx_proofs = CorrectnessProof::for_block(block, blobs);
        let rollup_row_proofs = CompletenessProof::from_filtered_block(block);

        (etx_proofs.0, rollup_row_proofs.0)
    }

    #[instrument(skip_all, err)]
    async fn send_transaction(&self, blob: &[u8]) -> Result<(), Self::Error> {
        debug!("Sending {} bytes of raw data to Celestia.", blob.len());

        let gas_limit = get_gas_limit_for_bytes(blob.len()) as u64;
        let fee = gas_limit * GAS_PRICE as u64;

        let blob = JsonBlob::new(self.rollup_namespace, blob.to_vec())?;
        info!("Submiting: {:?}", blob.commitment);

        let height = self
            .client
            .blob_submit(
                &[blob],
                SubmitOptions {
                    fee: Some(fee),
                    gas_limit: Some(gas_limit),
                },
            )
            .await?;
        info!(
            "Blob has been submitted to Celestia. block-height={}",
            height,
        );
        Ok(())
    }
}

// https://docs.celestia.org/learn/submit-data/#fees-and-gas-limits
fn get_gas_limit_for_bytes(n: usize) -> usize {
    let fixed_cost = 75000;

    let continuation_shares_needed =
        n.saturating_sub(FIRST_SPARSE_SHARE_CONTENT_SIZE) / CONTINUATION_SPARSE_SHARE_CONTENT_SIZE;
    let shares_needed = 1 + continuation_shares_needed + 1; // add one extra, pessimistic

    fixed_cost + shares_needed * SHARE_SIZE * GAS_PER_BYTE
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use celestia_types::nmt::Namespace;
    use celestia_types::{Blob as JsonBlob, NamespacedShares};
    use serde_json::json;
    use sov_rollup_interface::da::{BlockHeaderTrait, DaVerifier};
    use sov_rollup_interface::services::da::DaService;
    use wiremock::matchers::{bearer_token, body_json, method, path};
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};

    use super::default_request_timeout_seconds;
    use crate::da_service::{get_gas_limit_for_bytes, CelestiaService, DaServiceConfig, GAS_PRICE};
    use crate::parse_pfb_namespace;
    use crate::shares::NamespaceGroup;
    use crate::types::tests::{with_rollup_data, without_rollup_data};
    use crate::verifier::{CelestiaVerifier, RollupParams};

    const ROLLUP_ROWS_JSON: &str = with_rollup_data::ROLLUP_ROWS_JSON;
    const ETX_ROWS_JSON: &str = with_rollup_data::ETX_ROWS_JSON;

    #[test]
    fn test_get_pfbs() {
        let rows: NamespacedShares =
            serde_json::from_str(ETX_ROWS_JSON).expect("failed to deserialize pfb shares");

        let pfb_ns = NamespaceGroup::from(&rows);
        let pfbs = parse_pfb_namespace(pfb_ns).expect("failed to parse pfb shares");
        assert_eq!(pfbs.len(), 3);
    }

    #[test]
    fn test_get_rollup_data() {
        let rows: NamespacedShares =
            serde_json::from_str(ROLLUP_ROWS_JSON).expect("failed to deserialize pfb shares");

        let rollup_ns_group = NamespaceGroup::from(&rows);
        let mut blobs = rollup_ns_group.blobs();
        let first_blob = blobs
            .next()
            .expect("iterator should contain exactly one blob");

        // this is a batch submitted by sequencer, consisting of a single
        // "CreateToken" transaction, but we verify only length there to
        // not make this test depend on deserialization logic
        assert_eq!(first_blob.data().count(), 252);

        assert!(blobs.next().is_none());
    }

    // Last return value is namespace
    async fn setup_service(
        timeout_sec: Option<u64>,
    ) -> (MockServer, DaServiceConfig, CelestiaService, Namespace) {
        // Start a background HTTP server on a random local port
        let mock_server = MockServer::start().await;

        let timeout_sec = timeout_sec.unwrap_or_else(default_request_timeout_seconds);
        let config = DaServiceConfig {
            celestia_rpc_auth_token: "RPC_TOKEN".to_string(),
            celestia_rpc_address: mock_server.uri(),
            max_celestia_response_body_size: 120_000,
            celestia_rpc_timeout_seconds: timeout_sec,
        };
        let namespace = Namespace::new_v0(b"sov-test").unwrap();
        let da_service = CelestiaService::new(config.clone(), RollupParams { namespace }).await;

        (mock_server, config, da_service, namespace)
    }

    #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
    struct BasicJsonRpcRequest {
        jsonrpc: String,
        id: u64,
        method: String,
        params: serde_json::Value,
    }

    #[tokio::test]
    async fn test_submit_blob_correct() -> anyhow::Result<()> {
        let (mock_server, config, da_service, namespace) = setup_service(None).await;

        let blob = [1, 2, 3, 4, 5, 11, 12, 13, 14, 15];
        let gas_limit = get_gas_limit_for_bytes(blob.len());

        // TODO: Fee is hardcoded for now
        let expected_body = json!({
            "id": 0,
            "jsonrpc": "2.0",
            "method": "blob.Submit",
            "params": [
                [JsonBlob::new(namespace, blob.to_vec()).unwrap()],
                {
                    "GasLimit": gas_limit,
                    "Fee": gas_limit * GAS_PRICE,
                },
            ]
        });

        Mock::given(method("POST"))
            .and(path("/"))
            .and(bearer_token(config.celestia_rpc_auth_token))
            .and(body_json(&expected_body))
            .respond_with(|req: &Request| {
                let request: BasicJsonRpcRequest = serde_json::from_slice(&req.body).unwrap();
                let response_json = json!({
                    "jsonrpc": "2.0",
                    "id": request.id,
                    "result": 14, // just some block-height
                });

                ResponseTemplate::new(200)
                    .append_header("Content-Type", "application/json")
                    .set_body_json(response_json)
            })
            .up_to_n_times(1)
            .mount(&mock_server)
            .await;

        da_service.send_transaction(&blob).await?;

        Ok(())
    }

    #[tokio::test]
    async fn test_submit_blob_application_level_error() -> anyhow::Result<()> {
        // Our calculation of gas is off and gas limit exceeded, for example
        let (mock_server, _config, da_service, _namespace) = setup_service(None).await;

        let blob: Vec<u8> = vec![1, 2, 3, 4, 5, 11, 12, 13, 14, 15];

        // Do not check API token or expected body here.
        // Only interested in behaviour on response
        Mock::given(method("POST"))
            .and(path("/"))
            .respond_with(|req: &Request| {
                let request: BasicJsonRpcRequest = serde_json::from_slice(&req.body).unwrap();
                let response_json = json!({
                    "jsonrpc": "2.0",
                    "id": request.id,
                    "error": {
                        "code": 1,
                        "message": ": out of gas"
                    }
                });
                ResponseTemplate::new(200)
                    .append_header("Content-Type", "application/json")
                    .set_body_json(response_json)
            })
            .up_to_n_times(1)
            .mount(&mock_server)
            .await;

        let error = da_service
            .send_transaction(&blob)
            .await
            .unwrap_err()
            .to_string();

        assert!(error.contains("out of gas"));
        Ok(())
    }

    #[tokio::test]
    async fn test_submit_blob_internal_server_error() -> anyhow::Result<()> {
        let (mock_server, _config, da_service, _namespace) = setup_service(None).await;

        let error_response = ResponseTemplate::new(500).set_body_bytes("Internal Error".as_bytes());

        let blob: Vec<u8> = vec![1, 2, 3, 4, 5, 11, 12, 13, 14, 15];

        // Do not check API token or expected body here.
        // Only interested in behaviour on response
        Mock::given(method("POST"))
            .and(path("/"))
            .respond_with(error_response)
            .up_to_n_times(1)
            .mount(&mock_server)
            .await;

        let error = da_service
            .send_transaction(&blob)
            .await
            .unwrap_err()
            .to_string();

        assert!(error.contains(
            "Networking or low-level protocol error: Server returned an error status code: 500"
        ));
        Ok(())
    }

    #[tokio::test]
    // This test is slow now, but it can be fixed when
    // https://github.com/Sovereign-Labs/sovereign-sdk/issues/478 is implemented
    // Slower request timeout can be set
    async fn test_submit_blob_response_timeout() -> anyhow::Result<()> {
        let timeout = 1;
        let (mock_server, _config, da_service, _namespace) = setup_service(Some(timeout)).await;

        let response_json = json!({
            "jsonrpc": "2.0",
            "id": 0,
            "result": {
                "data": "122A0A282F365",
                "events": ["some event"],
                "gas_used": 70522,
                "gas_wanted": 133540,
                "height": 26,
                "logs":  [
                   "some log"
                ],
                "raw_log": "some raw logs",
                "txhash": "C9FEFD6D35FCC73F9E7D5C74E1D33F0B7666936876F2AD75E5D0FB2944BFADF2"
            }
        });

        let error_response = ResponseTemplate::new(200)
            .append_header("Content-Type", "application/json")
            .set_delay(Duration::from_secs(timeout) + Duration::from_millis(100))
            .set_body_json(response_json);

        let blob: Vec<u8> = vec![1, 2, 3, 4, 5, 11, 12, 13, 14, 15];

        // Do not check API token or expected body here.
        // Only interested in behaviour on response
        Mock::given(method("POST"))
            .and(path("/"))
            .respond_with(error_response)
            .up_to_n_times(1)
            .mount(&mock_server)
            .await;

        let error = da_service
            .send_transaction(&blob)
            .await
            .unwrap_err()
            .to_string();

        assert!(error.contains("Request timeout"));
        Ok(())
    }

    #[tokio::test]
    async fn verification_succeeds_for_correct_blocks() {
        let blocks = [
            with_rollup_data::filtered_block(),
            without_rollup_data::filtered_block(),
        ];

        for block in blocks {
            let (_, _, da_service, namespace) = setup_service(None).await;

            let txs = da_service.extract_relevant_blobs(&block);
            let (correctness_proof, completeness_proof) =
                da_service.get_extraction_proof(&block, &txs).await;

            let verifier = CelestiaVerifier::new(RollupParams { namespace });

            let validity_cond = verifier
                .verify_relevant_tx_list(&block.header, &txs, correctness_proof, completeness_proof)
                .unwrap();

            assert_eq!(validity_cond.prev_hash, *block.header.prev_hash().inner());
            assert_eq!(validity_cond.block_hash, *block.header.hash().inner());
        }
    }

    #[tokio::test]
    async fn verification_fails_if_tx_missing() {
        let block = with_rollup_data::filtered_block();
        let (_, _, da_service, namespace) = setup_service(None).await;

        let txs = da_service.extract_relevant_blobs(&block);
        let (correctness_proof, completeness_proof) =
            da_service.get_extraction_proof(&block, &txs).await;

        let verifier = CelestiaVerifier::new(RollupParams { namespace });

        // give verifier empty txs list
        let error = verifier
            .verify_relevant_tx_list(&block.header, &[], correctness_proof, completeness_proof)
            .unwrap_err();

        assert!(error.to_string().contains("Transaction missing"));
    }

    #[tokio::test]
    async fn verification_fails_if_not_all_etxs_are_proven() {
        let block = with_rollup_data::filtered_block();
        let (_, _, da_service, namespace) = setup_service(None).await;

        let txs = da_service.extract_relevant_blobs(&block);
        let (mut correctness_proof, completeness_proof) =
            da_service.get_extraction_proof(&block, &txs).await;

        // drop the proof for last etx
        correctness_proof.pop();

        let verifier = CelestiaVerifier::new(RollupParams { namespace });

        let error = verifier
            .verify_relevant_tx_list(&block.header, &txs, correctness_proof, completeness_proof)
            .unwrap_err();

        assert!(error.to_string().contains("not all blobs proven"));
    }

    #[tokio::test]
    async fn verification_fails_if_there_is_less_blobs_than_proofs() {
        let block = with_rollup_data::filtered_block();
        let (_, _, da_service, namespace) = setup_service(None).await;

        let txs = da_service.extract_relevant_blobs(&block);
        let (mut correctness_proof, completeness_proof) =
            da_service.get_extraction_proof(&block, &txs).await;

        // push one extra etx proof
        correctness_proof.push(correctness_proof[0].clone());

        let verifier = CelestiaVerifier::new(RollupParams { namespace });

        let error = verifier
            .verify_relevant_tx_list(&block.header, &txs, correctness_proof, completeness_proof)
            .unwrap_err();

        assert!(error.to_string().contains("more proofs than blobs"));
    }

    #[tokio::test]
    #[should_panic]
    async fn verification_fails_for_incorrect_namespace() {
        let block = with_rollup_data::filtered_block();
        let (_, _, da_service, _) = setup_service(None).await;

        let txs = da_service.extract_relevant_blobs(&block);
        let (correctness_proof, completeness_proof) =
            da_service.get_extraction_proof(&block, &txs).await;

        // create a verifier with a different namespace than the da_service
        let verifier = CelestiaVerifier::new(RollupParams {
            namespace: Namespace::new_v0(b"abc").unwrap(),
        });

        let _panics = verifier.verify_relevant_tx_list(
            &block.header,
            &txs,
            correctness_proof,
            completeness_proof,
        );
    }
}
