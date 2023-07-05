use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

use jsonrpsee::core::client::ClientT;
use jsonrpsee::http_client::{HeaderMap, HttpClient};
use nmt_rs::NamespaceId;
use sov_rollup_interface::da::CountedBufReader;
use sov_rollup_interface::services::da::DaService;
use tracing::{debug, info, span, Level};

// 0x736f762d74657374 = b"sov-test"
// For testing, use this NamespaceId (b"sov-test"):
// pub const ROLLUP_NAMESPACE: NamespaceId = NamespaceId([115, 111, 118, 45, 116, 101, 115, 116]);
use crate::{
    parse_pfb_namespace,
    share_commit::recreate_commitment,
    shares::{Blob, NamespaceGroup, Share},
    types::{ExtendedDataSquare, FilteredCelestiaBlock, Row, RpcNamespacedSharesResponse},
    utils::BoxError,
    verifier::{
        address::CelestiaAddress,
        proofs::{CompletenessProof, CorrectnessProof},
        CelestiaSpec, RollupParams, PFB_NAMESPACE,
    },
    BlobWithSender, CelestiaHeader, CelestiaHeaderResponse, DataAvailabilityHeader,
};

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
    let rollup_namespace_str = base64::encode(rollup_namespace).into();
    let rollup_shares_future = {
        let params: Vec<&serde_json::Value> = vec![dah, &rollup_namespace_str];
        client.request::<RpcNamespacedSharesResponse, _>("share.GetSharesByNamespace", params)
    };

    let etx_namespace_str = base64::encode(PFB_NAMESPACE).into();
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
}

fn default_rpc_addr() -> String {
    "http://localhost:11111/".into()
}

fn default_max_response_size() -> u32 {
    1024 * 1024 * 100 // 100 MB
}

impl DaService for CelestiaService {
    type RuntimeConfig = DaServiceConfig;

    type Spec = CelestiaSpec;

    type FilteredBlock = FilteredCelestiaBlock;

    type Future<T> = Pin<Box<dyn Future<Output = Result<T, Self::Error>> + Send>>;

    type Error = BoxError;

    fn new(config: Self::RuntimeConfig, chain_params: RollupParams) -> Self {
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
                .max_request_body_size(config.max_celestia_response_body_size) // 100 MB
                .build(&config.celestia_rpc_address)
        }
        .expect("Client initialization is valid");

        Self::with_client(client, chain_params.namespace)
    }

    fn get_finalized_at(&self, height: u64) -> Self::Future<Self::FilteredBlock> {
        let client = self.client.clone();
        let rollup_namespace = self.rollup_namespace;
        Box::pin(async move {
            let _span = span!(Level::TRACE, "fetching finalized block", height = height);
            // Fetch the header and relevant shares via RPC
            info!("Fetching header at height={}...", height);
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
            let rollup_rows = get_rows_containing_namespace(
                rollup_namespace,
                &dah,
                data_square.rows()?.into_iter(),
            )
            .await?;

            debug!("Decoding pfb protofbufs...");
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
        })
    }

    fn get_block_at(&self, height: u64) -> Self::Future<Self::FilteredBlock> {
        self.get_finalized_at(height)
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
                sender: CelestiaAddress(sender.as_bytes().to_vec()),
                hash: commitment,
            };

            output.push(blob_tx)
        }
        output
    }

    fn get_extraction_proof(
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

    fn send_transaction(&self, blob: &[u8]) -> <Self as DaService>::Future<()> {
        // https://node-rpc-docs.celestia.org/
        let client = self.client.clone();
        info!("Sending {} bytes of raw data to Celestia.", blob.len());
        // Take ownership of the blob so that the future is 'static.
        let blob = blob.to_vec();
        Box::pin(async move {
            let _response = client
                .request::<serde_json::Value, _>("state.SubmitTx", vec![blob])
                .await?;
            Ok::<(), BoxError>(())
        })
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
    use crate::parse_pfb_namespace;
    use crate::shares::{NamespaceGroup, Share};

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
}
