use std::collections::HashMap;
use std::str::FromStr;

use async_trait::async_trait;
use base64::engine::general_purpose::STANDARD as B64_ENGINE;
use base64::Engine;
use jsonrpsee::core::client::ClientT;
use jsonrpsee::core::params::ArrayParams;
use jsonrpsee::http_client::{HeaderMap, HttpClient};
use nmt_rs::NamespaceId;
use sov_rollup_interface::da::CountedBufReader;
use sov_rollup_interface::services::da::DaService;
use tracing::{debug, info, span, Level};

use crate::share_commit::recreate_commitment;
use crate::shares::{Blob, NamespaceGroup, Share};
use crate::types::{ExtendedDataSquare, FilteredCelestiaBlock, Row, RpcNamespacedSharesResponse};
use crate::utils::BoxError;
use crate::verifier::address::CelestiaAddress;
use crate::verifier::proofs::{CompletenessProof, CorrectnessProof};
use crate::verifier::{CelestiaSpec, RollupParams, PFB_NAMESPACE};
use crate::{
    parse_pfb_namespace, BlobWithSender, CelestiaHeader, CelestiaHeaderResponse,
    DataAvailabilityHeader,
};

// Approximate value, just to make it work.
const GAS_PER_BYTE: usize = 120;

#[derive(Debug, Clone)]
pub struct CelestiaService {
    client: HttpClient,
    rollup_namespace: NamespaceId,
}

impl CelestiaService {
    pub fn with_client(client: HttpClient, nid: NamespaceId) -> Self {
        Self {
            client,
            rollup_namespace: nid,
        }
    }
}

/// Fetch the rollup namespace shares and etx data. Returns a tuple `(rollup_shares, etx_shares)`
async fn fetch_needed_shares_by_header(
    rollup_namespace: NamespaceId,
    client: &HttpClient,
    header: &serde_json::Value,
) -> Result<(NamespaceGroup, NamespaceGroup), BoxError> {
    let dah = header
        .get("dah")
        .ok_or(BoxError::msg("missing dah in block header"))?;
    let rollup_namespace_str = B64_ENGINE.encode(rollup_namespace).into();
    let rollup_shares_future = {
        let params: Vec<&serde_json::Value> = vec![dah, &rollup_namespace_str];
        client.request::<RpcNamespacedSharesResponse, _>("share.GetSharesByNamespace", params)
    };

    let etx_namespace_str = B64_ENGINE.encode(PFB_NAMESPACE).into();
    let etx_shares_future = {
        let params: Vec<&serde_json::Value> = vec![dah, &etx_namespace_str];
        client.request::<RpcNamespacedSharesResponse, _>("share.GetSharesByNamespace", params)
    };

    let (rollup_shares_resp, etx_shares_resp) =
        tokio::join!(rollup_shares_future, etx_shares_future);

    let rollup_shares = NamespaceGroup::Sparse(
        rollup_shares_resp?
            .0
            .unwrap_or_default()
            .into_iter()
            .flat_map(|resp| resp.shares)
            .collect(),
    );
    let tx_data = NamespaceGroup::Compact(
        etx_shares_resp?
            .0
            .unwrap_or_default()
            .into_iter()
            .flat_map(|resp| resp.shares)
            .collect(),
    );

    Ok((rollup_shares, tx_data))
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

    type FilteredBlock = FilteredCelestiaBlock;

    type Error = BoxError;

    async fn get_finalized_at(&self, height: u64) -> Result<Self::FilteredBlock, Self::Error> {
        let client = self.client.clone();
        let rollup_namespace = self.rollup_namespace;

        let _span = span!(Level::TRACE, "fetching finalized block", height = height);
        // Fetch the header and relevant shares via RPC
        debug!("Fetching header at height={}...", height);
        let header = client
            .request::<serde_json::Value, _>("header.GetByHeight", vec![height])
            .await?;
        debug!(header_result = ?header);
        debug!("Fetching shares...");
        let (rollup_shares, tx_data) =
            fetch_needed_shares_by_header(rollup_namespace, &client, &header).await?;

        debug!("Fetching EDS...");
        // Fetch entire extended data square
        let data_square = client
            .request::<ExtendedDataSquare, _>(
                "share.GetEDS",
                vec![header
                    .get("dah")
                    .ok_or(BoxError::msg("missing 'dah' in block header"))?],
            )
            .await?;

        let unmarshalled_header: CelestiaHeaderResponse = serde_json::from_value(header)?;
        let dah: DataAvailabilityHeader = unmarshalled_header.dah.try_into()?;
        debug!("Parsing namespaces...");
        // Parse out all of the rows containing etxs
        let etx_rows =
            get_rows_containing_namespace(PFB_NAMESPACE, &dah, data_square.rows()?.into_iter())
                .await?;
        // Parse out all of the rows containing rollup data
        let rollup_rows =
            get_rows_containing_namespace(rollup_namespace, &dah, data_square.rows()?.into_iter())
                .await?;

        debug!("Decoding pfb protobufs...");
        // Parse out the pfds and store them for later retrieval
        let pfds = parse_pfb_namespace(tx_data)?;
        let mut pfd_map = HashMap::new();
        for tx in pfds {
            for (idx, nid) in tx.0.namespace_ids.iter().enumerate() {
                if nid == &rollup_namespace.0[..] {
                    // TODO: Retool this map to avoid cloning txs
                    pfd_map.insert(tx.0.share_commitments[idx].clone(), tx.clone());
                }
            }
        }

        let filtered_block = FilteredCelestiaBlock {
            header: CelestiaHeader::new(dah, unmarshalled_header.header.into()),
            rollup_data: rollup_shares,
            relevant_pfbs: pfd_map,
            rollup_rows,
            pfb_rows: etx_rows,
        };

        Ok::<Self::FilteredBlock, BoxError>(filtered_block)
    }

    async fn get_block_at(&self, height: u64) -> Result<Self::FilteredBlock, Self::Error> {
        self.get_finalized_at(height).await
    }

    fn extract_relevant_txs(
        &self,
        block: &Self::FilteredBlock,
    ) -> Vec<<Self::Spec as sov_rollup_interface::da::DaSpec>::BlobTransaction> {
        let mut output = Vec::new();
        for blob_ref in block.rollup_data.blobs() {
            let commitment = recreate_commitment(block.square_size(), blob_ref.clone())
                .expect("blob must be valid");
            let sender = block
                .relevant_pfbs
                .get(&commitment[..])
                .expect("blob must be relevant")
                .0
                .signer
                .clone();

            let blob: Blob = blob_ref.into();

            let blob_tx = BlobWithSender {
                blob: CountedBufReader::new(blob.into_iter()),
                sender: CelestiaAddress::from_str(&sender).expect("Incorrect sender address"),
                hash: commitment,
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
        let rollup_row_proofs =
            CompletenessProof::from_filtered_block(block, self.rollup_namespace);

        (etx_proofs.0, rollup_row_proofs.0)
    }

    async fn send_transaction(&self, blob: &[u8]) -> Result<(), Self::Error> {
        // https://node-rpc-docs.celestia.org/
        let client = self.client.clone();
        debug!("Sending {} bytes of raw data to Celestia.", blob.len());
        let fee: u64 = 2000;
        let namespace = self.rollup_namespace.0.to_vec();
        let blob = blob.to_vec();
        // We factor extra share to be occupied for namespace, which is pessimistic
        let gas_limit = get_gas_limit_for_bytes(blob.len());

        let mut params = ArrayParams::new();
        params.insert(namespace)?;
        params.insert(blob)?;
        params.insert(fee.to_string())?;
        params.insert(gas_limit)?;
        // Note, we only deserialize what we can use, other fields might be left over
        let response = client
            .request::<CelestiaBasicResponse, _>("state.SubmitPayForBlob", params)
            .await?;
        if !response.is_success() {
            anyhow::bail!("Error returned from Celestia node: {:?}", response);
        }
        debug!("Response after submitting blob: {:?}", response);
        info!(
            "Blob has been submitted to Celestia. tx-hash={}",
            response.tx_hash,
        );
        Ok::<(), BoxError>(())
    }
}

fn get_gas_limit_for_bytes(n: usize) -> usize {
    (n + 512) * GAS_PER_BYTE + 1060
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct CelestiaBasicResponse {
    raw_log: String,
    #[serde(rename = "code")]
    error_code: Option<u64>,
    #[serde(rename = "txhash")]
    tx_hash: String,
    gas_wanted: u64,
    gas_used: u64,
}

impl CelestiaBasicResponse {
    /// We assume that absence of `code` indicates that request was successful
    pub fn is_success(&self) -> bool {
        self.error_code.is_none()
    }
}

async fn get_rows_containing_namespace(
    nid: NamespaceId,
    dah: &DataAvailabilityHeader,
    data_square_rows: impl Iterator<Item = &[Share]>,
) -> Result<Vec<Row>, BoxError> {
    let mut output = vec![];

    for (row, root) in data_square_rows.zip(dah.row_roots.iter()) {
        if root.contains(nid) {
            output.push(Row {
                shares: row.to_vec(),
                root: root.clone(),
            })
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use nmt_rs::NamespaceId;
    use serde_json::json;
    use sov_rollup_interface::services::da::DaService;
    use wiremock::matchers::{bearer_token, body_json, method, path};
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};

    use super::default_request_timeout_seconds;
    use crate::da_service::{CelestiaService, DaServiceConfig};
    use crate::parse_pfb_namespace;
    use crate::shares::{NamespaceGroup, Share};
    use crate::verifier::RollupParams;

    const SERIALIZED_PFB_SHARES: &str = r#"["AAAAAAAAAAQBAAABRQAAABHDAgq3AgqKAQqHAQogL2NlbGVzdGlhLmJsb2IudjEuTXNnUGF5Rm9yQmxvYnMSYwovY2VsZXN0aWExemZ2cnJmYXE5dWQ2Zzl0NGt6bXNscGYyNHlzYXhxZm56ZWU1dzkSCHNvdi10ZXN0GgEoIiCB8FoaUuOPrX2wFBbl4MnWY3qE72tns7sSY8xyHnQtr0IBABJmClAKRgofL2Nvc21vcy5jcnlwdG8uc2VjcDI1NmsxLlB1YktleRIjCiEDmXaTf6RVIgUVdG0XZ6bqecEn8jWeAi+LjzTis5QZdd4SBAoCCAEYARISCgwKBHV0aWESBDIwMDAQgPEEGkAhq2CzD1DqxsVXIriANXYyLAmJlnnt8YTNXiwHgMQQGUbl65QUe37UhnbNVrOzDVYK/nQV9TgI+5NetB2JbIz6EgEBGgRJTkRYAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="]"#;
    const SERIALIZED_ROLLUP_DATA_SHARES: &str = r#"["c292LXRlc3QBAAAAKHsia2V5IjogInRlc3RrZXkiLCAidmFsdWUiOiAidGVzdHZhbHVlIn0AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="]"#;

    #[test]
    fn test_get_pfbs() {
        // the following test case is taken from arabica-6, block 275345
        let shares: Vec<Share> =
            serde_json::from_str(SERIALIZED_PFB_SHARES).expect("failed to deserialize pfb shares");

        assert_eq!(shares.len(), 1);

        let pfb_ns = NamespaceGroup::Compact(shares);
        let pfbs = parse_pfb_namespace(pfb_ns).expect("failed to parse pfb shares");
        assert_eq!(pfbs.len(), 1);
    }

    #[test]
    fn test_get_rollup_data() {
        let shares: Vec<Share> = serde_json::from_str(SERIALIZED_ROLLUP_DATA_SHARES)
            .expect("failed to deserialize pfb shares");

        let rollup_ns_group = NamespaceGroup::Sparse(shares);
        let mut blobs = rollup_ns_group.blobs();
        let first_blob = blobs
            .next()
            .expect("iterator should contain exactly one blob");

        let found_data: Vec<u8> = first_blob.data().collect();
        assert_eq!(
            found_data,
            r#"{"key": "testkey", "value": "testvalue"}"#.as_bytes()
        );

        assert!(blobs.next().is_none());
    }

    // Last return value is namespace
    async fn setup_service(
        timeout_sec: Option<u64>,
    ) -> (MockServer, DaServiceConfig, CelestiaService, [u8; 8]) {
        // Start a background HTTP server on a random local port
        let mock_server = MockServer::start().await;

        let timeout_sec = timeout_sec.unwrap_or_else(default_request_timeout_seconds);
        let config = DaServiceConfig {
            celestia_rpc_auth_token: "RPC_TOKEN".to_string(),
            celestia_rpc_address: mock_server.uri(),
            max_celestia_response_body_size: 120_000,
            celestia_rpc_timeout_seconds: timeout_sec,
        };
        let namespace = [9u8; 8];
        let da_service = CelestiaService::new(
            config.clone(),
            RollupParams {
                namespace: NamespaceId(namespace),
            },
        )
        .await;

        (mock_server, config, da_service, namespace)
    }

    #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
    struct BasicJsonRpcResponse {
        jsonrpc: String,
        id: u64,
        method: String,
        params: serde_json::Value,
    }

    #[tokio::test]
    async fn test_submit_blob_correct() -> anyhow::Result<()> {
        let (mock_server, config, da_service, namespace) = setup_service(None).await;

        let blob: Vec<u8> = vec![1, 2, 3, 4, 5, 11, 12, 13, 14, 15];

        // TODO: Fee is hardcoded for now
        let expected_body = json!({
            "id": 0,
            "jsonrpc": "2.0",
            "method": "state.SubmitPayForBlob",
            "params": [
                namespace,
                blob,
                "2000",
                63700
            ]
        });

        Mock::given(method("POST"))
            .and(path("/"))
            .and(bearer_token(config.celestia_rpc_auth_token))
            .and(body_json(&expected_body))
            .respond_with(|req: &Request| {
                let request: BasicJsonRpcResponse = serde_json::from_slice(&req.body).unwrap();
                let response_json = json!({
                    "jsonrpc": "2.0",
                    "id": request.id,
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
                let request: BasicJsonRpcResponse = serde_json::from_slice(&req.body).unwrap();
                let response_json = json!({
                    "jsonrpc": "2.0",
                    "id": request.id,
                    "result": {
                        "code": 11,
                        "codespace": "sdk",
                        "gas_used": 10_000,
                        "gas_wanted": 12_000,
                        "raw_log": "out of gas in location: ReadFlat; gasWanted: 10, gasUsed: 1000: out of gas",
                        "txhash": "C9FEFD6D35FCC73F9E7D5C74E1D33F0B7666936876F2AD75E5D0FB2944BFADF2"
                    }
                });
                ResponseTemplate::new(200)
                    .append_header("Content-Type", "application/json")
                    .set_body_json(response_json)
            })
            .up_to_n_times(1)
            .mount(&mock_server)
            .await;

        let result = da_service.send_transaction(&blob).await;

        assert!(result.is_err());
        let error_string = result.err().unwrap().to_string();
        assert!(error_string.contains("Error returned from Celestia node:"));
        assert!(error_string.contains(
            "out of gas in location: ReadFlat; gasWanted: 10, gasUsed: 1000: out of gas"
        ));
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

        let result = da_service.send_transaction(&blob).await;

        assert!(result.is_err());
        let error_string = result.err().unwrap().to_string();
        assert!(error_string.contains(
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

        let result = da_service.send_transaction(&blob).await;

        assert!(result.is_err());
        let error_string = result.err().unwrap().to_string();
        assert!(error_string.contains("Request timeout"));
        Ok(())
    }
}
