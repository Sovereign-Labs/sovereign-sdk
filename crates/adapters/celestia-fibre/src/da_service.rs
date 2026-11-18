use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context;
use async_trait::async_trait;
use backon::Retryable;
use celestia_client::api::fibre::{BlobID, DownloadOptions, FibreClient, FibreClientConfig};
use celestia_client::tx::{SigningKey, TxConfig};
use celestia_client::types::namespace_data::NamespaceData;
use celestia_client::types::nmt::Namespace;
use celestia_client::types::state::AddressTrait;
use celestia_client::types::state::ErrorCode;
use celestia_grpc::grpc::{BroadcastMode, TxStatus};
use celestia_grpc::BroadcastedTx;
use sov_celestia_adapter::types::TmHash;
use sov_celestia_adapter::verifier::address::CelestiaAddress;
use sov_celestia_adapter::verifier::RollupParams;
use sov_celestia_adapter::CelestiaHeader;
use sov_rollup_interface::common::HexHash;
use sov_rollup_interface::da::{
    BlobReaderTrait, BlockHeaderTrait, DaProof, RelevantBlobs, RelevantProofs,
};
use sov_rollup_interface::node::da::{
    run_maybe_retryable_async_fn_with_retries, DaService, MaybeRetryable, SlotData,
    SubmitBlobReceipt,
};
use tokio::sync::oneshot;

use crate::commitment::prove_rows_prefix;
use crate::metrics::{self, Operation};
use crate::namespace::{prove_namespace, verify_namespace_proof, FIBRE_BLOB_VERSION};
use crate::{
    CelestiaFibreConfig, CelestiaFibreSpec, CelestiaFibreVerifier, FibreBlob, FibreNamespaceProof,
    FibreRowsProof,
};

/// Celestia block header, the proofs of the rollup namespaces' sequences, and the Fibre blobs
/// they reference, already downloaded from validators.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FilteredFibreBlock {
    pub(crate) header: CelestiaHeader,
    pub(crate) batch_namespace_proof: FibreNamespaceProof,
    pub(crate) batch_blobs: Vec<DownloadedFibreBlob>,
    pub(crate) proof_namespace_proof: FibreNamespaceProof,
    pub(crate) proof_blobs: Vec<DownloadedFibreBlob>,
}

/// Full data of a Fibre blob referenced from the rollup namespace.
///
/// Unlike [`FibreBlob`], which keeps only the read prefix when serialized, this always
/// (de)serializes the whole data.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct DownloadedFibreBlob {
    sender: CelestiaAddress,
    commitment: [u8; 32],
    data: bytes::Bytes,
}

impl SlotData for FilteredFibreBlock {
    type BlockHeader = CelestiaHeader;

    fn hash(&self) -> [u8; 32] {
        SlotData::hash(&self.header)
    }

    fn header(&self) -> &Self::BlockHeader {
        &self.header
    }

    fn timestamp(&self) -> sov_rollup_interface::da::Time {
        SlotData::timestamp(&self.header)
    }
}

/// [`DaService`] that reads blob references from a bridge node and blob data from Fibre.
#[derive(Clone)]
pub struct CelestiaFibreDaService {
    client: Arc<celestia_client::Client>,
    grpc: celestia_grpc::GrpcClient,
    fibre: Arc<FibreClient>,
    signing_key: Option<SigningKey>,
    signer_address: Option<CelestiaAddress>,
    rollup_batch_namespace: Namespace,
    rollup_proof_namespace: Namespace,
    request_timeout: Duration,
    backoff_policy: backon::ExponentialBuilder,
}

impl CelestiaFibreDaService {
    /// Connects to the bridge node and consensus gRPC. Read-only unless `signer_private_key` is set.
    pub async fn new(
        config: CelestiaFibreConfig,
        chain_params: RollupParams,
    ) -> anyhow::Result<Self> {
        tracing::info!(?config, "Initializing Celestia Fibre Adapter");
        config.validate()?;

        let mut rpc = celestia_client::Client::builder()
            .rpc_url(&config.rpc_url)
            .timeout(config.api_request_timeout());
        if let Some(token) = &config.rpc_auth_token {
            rpc = rpc.rpc_auth_token(token);
        }
        // `rpc_url` stays the preferred endpoint; the fallbacks are appended after it.
        if !config.rpc_fallback_endpoints.is_empty() {
            rpc = rpc.rpc_endpoints(config.rpc_fallback_endpoints.iter().map(|endpoint| {
                let mut rpc_endpoint = celestia_client::RpcEndpoint::new(endpoint.url.clone());
                if let Some(token) = &endpoint.token {
                    rpc_endpoint = rpc_endpoint.auth_token(token);
                }
                rpc_endpoint
            }));
            if let Some(interval) = config.rpc_health_check_interval_secs {
                rpc = rpc.rpc_health_check_interval(Duration::from_secs(interval.get()));
            }
            if let Some(max_head_age) = config.rpc_max_head_age_secs {
                rpc = rpc.rpc_max_head_age(Duration::from_secs(max_head_age.get()));
            }
        }
        let client = rpc
            .build()
            .await
            .context("Failed to build celestia-client")?;

        let mut grpc = celestia_grpc::GrpcClient::builder()
            .endpoint(grpc_endpoint(&config.grpc_url, &config.grpc_auth_token))
            .timeout(config.api_request_timeout());
        for endpoint in &config.grpc_fallback_endpoints {
            grpc = grpc.endpoint(grpc_endpoint(&endpoint.url, &endpoint.token));
        }
        let mut signing_key = None;
        if let Some(key_hex) = &config.signer_private_key {
            let key_bytes = hex::decode(key_hex).context("`signer_private_key` is not hex")?;
            signing_key = Some(SigningKey::from_slice(&key_bytes).context("Invalid signer key")?);
            grpc = grpc.private_key(&key_bytes);
        }
        let grpc = grpc.build().context("Failed to build gRPC client")?;

        let chain_id = grpc.chain_id().await.context("Failed to fetch chain id")?;
        let fibre_grpc = match &config.fibre_grpc_url {
            Some(url) => celestia_grpc::GrpcClient::builder()
                .endpoint(grpc_endpoint(url, &config.fibre_grpc_auth_token))
                .timeout(config.api_request_timeout())
                .build()
                .context("Failed to build Fibre gRPC client")?,
            None => grpc.clone(),
        };
        let fibre = fibre_client(
            fibre_grpc,
            FibreClientConfig::new(chain_id.as_str())?,
            config.fibre_request_timeout(),
        )?;

        let signer_address = grpc
            .get_account_address()
            .map(|address| CelestiaAddress::try_from(address.as_bytes()))
            .transpose()?;
        if signer_address.is_none() {
            tracing::info!("CelestiaFibreDaService is read-only and won't be able to submit blobs");
        }

        Ok(Self {
            client: Arc::new(client),
            grpc,
            fibre: Arc::new(fibre),
            signing_key,
            signer_address,
            rollup_batch_namespace: chain_params.rollup_batch_namespace,
            rollup_proof_namespace: chain_params.rollup_proof_namespace,
            request_timeout: config.request_timeout(),
            backoff_policy: config.backoff_policy(),
        })
    }

    /// Runs a Celestia node call with the same timeout and retries as `sov-celestia-adapter`:
    /// every attempt is bounded by `request_timeout_secs`, and every failure is retried as
    /// transient according to the `backoff_*` config parameters. Each attempt is reported as a
    /// metric of `operation`.
    async fn call_with_retries<T, E, Fut>(
        &self,
        method_name: &str,
        operation: Operation,
        call: impl Fn() -> Fut,
    ) -> anyhow::Result<T>
    where
        Fut: Future<Output = Result<T, E>>,
        E: Into<anyhow::Error>,
    {
        run_maybe_retryable_async_fn_with_retries(
            self.backoff_policy,
            || async {
                let start = Instant::now();
                let result = tokio::time::timeout(self.request_timeout, call()).await;
                metrics::track(
                    operation,
                    start.elapsed(),
                    matches!(result, Ok(Ok(_))),
                    None,
                );
                match result {
                    Ok(Ok(response)) => Ok(response),
                    Ok(Err(error)) => Err(MaybeRetryable::Transient(error.into())),
                    Err(_) => Err(MaybeRetryable::Transient(anyhow::anyhow!("await timeout"))),
                }
            },
            method_name,
        )
        .await
    }

    async fn get_namespace_data(
        &self,
        height: u64,
        namespace: Namespace,
    ) -> anyhow::Result<NamespaceData> {
        self.call_with_retries(
            "get_namespace_data",
            Operation::GetNamespaceData,
            || async {
                self.client
                    .share()
                    .get_namespace_data(height, namespace)
                    .await
            },
        )
        .await
    }

    /// Proves the sequences of `namespace`, selects the Fibre references among them by verifying
    /// that proof like the verifier does, and downloads the blobs they reference.
    async fn download_fibre_blobs(
        &self,
        header: &CelestiaHeader,
        namespace: Namespace,
        namespace_data: NamespaceData,
    ) -> anyhow::Result<(FibreNamespaceProof, Vec<DownloadedFibreBlob>)> {
        let height = header.height();
        let namespace_proof = prove_namespace(header, namespace, namespace_data)
            .with_context(|| format!("Invalid namespace data at height {height}"))?;
        let refs = verify_namespace_proof(header, namespace, &namespace_proof)
            .with_context(|| format!("Invalid namespace proof at height {height}"))?;

        let mut fibre_blobs = Vec::with_capacity(refs.len());
        // TODO: downloads run sequentially, parallelize if blocks carry many Fibre blobs.
        for fibre_ref in refs {
            let blob_id = BlobID::new(FIBRE_BLOB_VERSION, fibre_ref.commitment);
            // The blob is stored by the validators of the block that references it, which can
            // differ from the head validator set when the node is catching up.
            let options = || DownloadOptions {
                height: Some(height),
                ..DownloadOptions::default()
            };
            // Validators signed that they store this blob, so a failure is transient. A single
            // unresponsive validator delays an attempt by at most `fibre_request_timeout_secs`:
            // its request then fails, and the client asks another validator.
            let download = || async {
                let start = Instant::now();
                let result = self.fibre.download(&blob_id, options()).await;
                let bytes = result.as_ref().ok().map(|blob| blob.data().len());
                metrics::track(
                    Operation::FibreDownload,
                    start.elapsed(),
                    result.is_ok(),
                    bytes,
                );
                result
            };
            let downloaded = download
                .retry(fibre_backoff())
                .notify(|error, delay| {
                    tracing::warn!(
                        height,
                        commitment = %HexHash::new(fibre_ref.commitment),
                        %error,
                        ?delay,
                        "Failed to download Fibre blob, retrying"
                    );
                })
                .await
                .with_context(|| format!("Failed to download Fibre blob at height {height}"))?;
            fibre_blobs.push(DownloadedFibreBlob {
                sender: fibre_ref.sender,
                commitment: fibre_ref.commitment,
                data: downloaded.into_data(),
            });
        }
        Ok((namespace_proof, fibre_blobs))
    }

    async fn submit(
        &self,
        data: &[u8],
        namespace: Namespace,
    ) -> anyhow::Result<SubmitBlobReceipt<TmHash>> {
        let (Some(signing_key), Some(account)) =
            (&self.signing_key, self.grpc.get_account_address())
        else {
            anyhow::bail!("Signer must be set for submitting blobs");
        };

        // A failed upload returns no payment promise, so retrying it can't make us pay twice.
        // Each attempt signs a new promise. Validators that stored the shards of a failed attempt
        // keep its promise and could charge it after it times out.
        let upload = || async {
            let start = Instant::now();
            let result = self
                .fibre
                .upload_and_prepare(signing_key, namespace, data, &account)
                .await;
            metrics::track(
                Operation::FibreUpload,
                start.elapsed(),
                result.is_ok(),
                Some(data.len()),
            );
            result
        };
        let msg = upload
            .retry(fibre_backoff())
            .notify(|error, delay| {
                tracing::warn!(%error, ?delay, "Failed to upload Fibre blob, retrying");
            })
            .await
            .context("Failed to upload Fibre blob")?;
        let commitment: [u8; 32] = msg
            .payment_promise
            .as_ref()
            .context("MsgPayForFibre has no payment promise")?
            .commitment
            .as_slice()
            .try_into()
            .context("Fibre commitment is not 32 bytes")?;

        // A broadcast attempt that times out may still reach the mempool, and the retry then
        // carries the same payment promise. The chain settles a promise only once, and
        // `confirm_pay_for_fibre` accepts the "already processed" rejection of the duplicate.
        let submitted = self
            .call_with_retries(
                "broadcast_pay_for_fibre",
                Operation::SubmitPayForFibre,
                || async {
                    self.grpc
                        .broadcast_message(msg.clone(), TxConfig::default())
                        .await
                },
            )
            .await?;
        let tx = submitted.tx_ref();
        let da_height = self.confirm_pay_for_fibre(tx).await?;
        tracing::info!(
            ?da_height,
            bytes = data.len(),
            commitment = %HexHash::new(commitment),
            tx_hash = %tx.hash,
            "Blob has been submitted to Celestia Fibre"
        );

        Ok(SubmitBlobReceipt {
            blob_hash: HexHash::new(commitment),
            da_transaction_id: TmHash(tx.hash),
        })
    }

    /// Waits until a broadcast `MsgPayForFibre` is committed and returns its height, or `None`
    /// if a duplicate of it settled the payment promise first.
    ///
    /// Unlike [`celestia_grpc::SubmittedTx::confirm`], a failed status query is retried instead
    /// of failing the submission: the blob sender would then upload the blob again and pay for it
    /// a second time. The tx is never re-signed, because a new tx would carry the same payment
    /// promise. Like `confirm`, an evicted tx is broadcast again with the same signed bytes.
    async fn confirm_pay_for_fibre(&self, tx: &BroadcastedTx) -> anyhow::Result<Option<u64>> {
        let hash = tx.hash;
        loop {
            let status = self
                .call_with_retries("tx_status", Operation::TxStatus, || async {
                    self.grpc.tx_status(hash).await
                })
                .await?;
            match status.status {
                TxStatus::Pending => tokio::time::sleep(TX_CONFIRMATION_INTERVAL).await,
                TxStatus::Committed if status.execution_code == ErrorCode::Success => {
                    return Ok(Some(status.height.value()));
                }
                TxStatus::Committed | TxStatus::Rejected
                    if status.error.contains(PROMISE_ALREADY_PROCESSED) =>
                {
                    tracing::warn!(
                        %hash,
                        error = status.error,
                        "Payment promise was settled by a duplicate MsgPayForFibre"
                    );
                    return Ok(None);
                }
                TxStatus::Committed | TxStatus::Rejected => anyhow::bail!(
                    "MsgPayForFibre {hash} failed with {:?} ({:?}): {}",
                    status.status,
                    status.execution_code,
                    status.error
                ),
                TxStatus::Evicted | TxStatus::Unknown => {
                    tracing::warn!(%hash, status = ?status.status, "Broadcasting MsgPayForFibre again");
                    let response = self
                        .grpc
                        .broadcast_tx(tx.tx.clone(), BroadcastMode::Sync)
                        .await
                        .with_context(|| {
                            format!("Failed to broadcast MsgPayForFibre {hash} again")
                        })?;
                    anyhow::ensure!(
                        response.code == ErrorCode::Success,
                        "Failed to broadcast MsgPayForFibre {hash} again: {:?}: {}",
                        response.code,
                        response.raw_log
                    );
                }
            }
        }
    }
}

/// How often `confirm_pay_for_fibre` polls the status of a pending tx. Same as `celestia-grpc`.
const TX_CONFIRMATION_INTERVAL: Duration = Duration::from_millis(500);

/// Error of a `MsgPayForFibre` whose payment promise was already settled
/// (celestia-app `x/fibre/keeper/keeper.go`).
const PROMISE_ALREADY_PROCESSED: &str = "payment promise has already been processed";

fn grpc_endpoint(url: &str, auth_token: &Option<String>) -> celestia_grpc::Endpoint {
    let endpoint = celestia_grpc::Endpoint::new(url.to_owned());
    match auth_token {
        Some(token) => endpoint.metadata("x-token", token),
        None => endpoint,
    }
}

/// Builds the [`FibreClient`] like [`FibreClient::from_grpc_client`], but with a configurable
/// timeout on every request to a validator, connection setup included. `from_grpc_client` leaves
/// it at `celestia-grpc`'s default of 30 seconds.
fn fibre_client(
    grpc: celestia_grpc::GrpcClient,
    config: FibreClientConfig,
    validator_request_timeout: Duration,
) -> anyhow::Result<FibreClient> {
    let host_registry = Arc::new(celestia_fibre::GrpcHostRegistry::new(grpc.clone()));
    let connector = celestia_fibre::GrpcValidatorConnector::new(host_registry, config.chain_id())
        .with_request_timeout(validator_request_timeout);
    Ok(celestia_fibre::FibreClient::builder()
        .config(config)
        .set_getter(celestia_fibre::GrpcSetGetter::new(grpc))
        .connector(connector)
        .build()?)
}

/// Retries for a Fibre download or upload: about a minute in total before it fails.
fn fibre_backoff() -> backon::ExponentialBuilder {
    backon::ExponentialBuilder::default()
        .with_min_delay(Duration::from_millis(500))
        .with_max_delay(Duration::from_secs(16))
        .with_max_times(8)
}

/// Builds the row proof for the read prefix of each downloaded blob; `read_lens` follows the same
/// order. Validators checked the blobs against their commitments, so a failure here is a bug in
/// this crate. The blob then gets `None`, which makes the block unprovable without stopping the
/// node.
fn prove_read_prefixes(
    blobs: &[DownloadedFibreBlob],
    read_lens: &[usize],
) -> Vec<Option<FibreRowsProof>> {
    assert_eq!(
        blobs.len(),
        read_lens.len(),
        "Bug: relevant blobs were not extracted from this block"
    );
    blobs
        .iter()
        .zip(read_lens)
        .map(
            |(blob, read_len)| match prove_rows_prefix(&blob.commitment, &blob.data, *read_len) {
                Ok(proof) => Some(proof),
                Err(error) => {
                    tracing::error!(
                        commitment = %HexHash::new(blob.commitment),
                        %error,
                        "Failed to build Fibre row proof, this block can't be proven"
                    );
                    None
                }
            },
        )
        .collect()
}

#[async_trait]
impl DaService for CelestiaFibreDaService {
    type Spec = CelestiaFibreSpec;
    type Config = CelestiaFibreConfig;
    type Verifier = CelestiaFibreVerifier;
    type FilteredBlock = FilteredFibreBlock;
    type Error = anyhow::Error;

    async fn get_block_at(&self, height: u64) -> Result<Self::FilteredBlock, Self::Error> {
        let (header, batch_namespace_data, proof_namespace_data) = tokio::try_join!(
            self.get_block_header_at(height),
            self.get_namespace_data(height, self.rollup_batch_namespace),
            self.get_namespace_data(height, self.rollup_proof_namespace),
        )?;
        let ((batch_namespace_proof, batch_blobs), (proof_namespace_proof, proof_blobs)) = tokio::try_join!(
            self.download_fibre_blobs(&header, self.rollup_batch_namespace, batch_namespace_data),
            self.download_fibre_blobs(&header, self.rollup_proof_namespace, proof_namespace_data),
        )?;
        Ok(FilteredFibreBlock {
            header,
            batch_namespace_proof,
            batch_blobs,
            proof_namespace_proof,
            proof_blobs,
        })
    }

    async fn get_block_header_at(&self, height: u64) -> Result<CelestiaHeader, Self::Error> {
        let header = self
            .call_with_retries("get_block_header_at", Operation::GetBlockHeader, || async {
                let header = self.client.header().get_by_height(height).await?;
                // Same check as `sov-celestia-adapter`: a header of another height is retried.
                let received = header.header.height.value();
                anyhow::ensure!(
                    received == height,
                    "Received wrong height {received}, when requested {height}"
                );
                Ok::<_, anyhow::Error>(header)
            })
            .await?;
        Ok(header.into())
    }

    async fn get_last_finalized_block_header(&self) -> Result<CelestiaHeader, Self::Error> {
        // Tendermint has instant finality.
        self.get_head_block_header().await
    }

    async fn get_head_block_header(&self) -> Result<CelestiaHeader, Self::Error> {
        let header = self
            .call_with_retries(
                "get_head_block_header",
                Operation::GetHeadBlockHeader,
                || async { self.client.header().network_head().await },
            )
            .await?;
        Ok(header.into())
    }

    fn extract_relevant_blobs(&self, block: &Self::FilteredBlock) -> RelevantBlobs<FibreBlob> {
        let to_blobs = |blobs: &[DownloadedFibreBlob]| {
            blobs
                .iter()
                .map(|blob| FibreBlob::new(blob.sender, blob.commitment, blob.data.clone()))
                .collect()
        };
        RelevantBlobs {
            proof_blobs: to_blobs(&block.proof_blobs),
            batch_blobs: to_blobs(&block.batch_blobs),
        }
    }

    async fn get_extraction_proof(
        &self,
        block: &Self::FilteredBlock,
        blobs: &RelevantBlobs<FibreBlob>,
    ) -> RelevantProofs<Vec<Option<FibreRowsProof>>, FibreNamespaceProof> {
        let read_lens = |blobs: &[FibreBlob]| -> Vec<usize> {
            blobs
                .iter()
                .map(|blob| blob.verified_data().len())
                .collect()
        };
        // Cloning only bumps the reference counts of the blob data.
        let batch = (block.batch_blobs.clone(), read_lens(&blobs.batch_blobs));
        let proof = (block.proof_blobs.clone(), read_lens(&blobs.proof_blobs));

        // Re-encoding blobs is CPU-heavy, keep it off the async workers.
        let (batch, proof) = tokio::task::spawn_blocking(move || {
            (
                prove_read_prefixes(&batch.0, &batch.1),
                prove_read_prefixes(&proof.0, &proof.1),
            )
        })
        .await
        .expect("Fibre row proof generation panicked");

        RelevantProofs {
            batch: DaProof {
                inclusion_proof: batch,
                completeness_proof: block.batch_namespace_proof.clone(),
            },
            proof: DaProof {
                inclusion_proof: proof,
                completeness_proof: block.proof_namespace_proof.clone(),
            },
        }
    }

    async fn send_transaction(
        &self,
        blob: &[u8],
    ) -> oneshot::Receiver<Result<SubmitBlobReceipt<TmHash>, Self::Error>> {
        let (tx, rx) = oneshot::channel();
        let res = self
            .submit(blob, self.rollup_batch_namespace)
            .await
            .context("Batch submit");
        // UNWRAP: Not possible because the receiver is in the scope still
        tx.send(res).unwrap();
        rx
    }

    async fn send_proof(
        &self,
        aggregated_proof: &[u8],
    ) -> oneshot::Receiver<Result<SubmitBlobReceipt<TmHash>, Self::Error>> {
        let (tx, rx) = oneshot::channel();
        let res = self
            .submit(aggregated_proof, self.rollup_proof_namespace)
            .await
            .context("Proof submit");
        // UNWRAP: Not possible because the receiver is in the scope still
        tx.send(res).unwrap();
        rx
    }

    async fn get_proofs_at(&self, height: u64) -> Result<Vec<Vec<u8>>, Self::Error> {
        let (header, namespace_data) = tokio::try_join!(
            self.get_block_header_at(height),
            self.get_namespace_data(height, self.rollup_proof_namespace),
        )?;
        let (_, blobs) = self
            .download_fibre_blobs(&header, self.rollup_proof_namespace, namespace_data)
            .await?;
        Ok(blobs.into_iter().map(|blob| blob.data.to_vec()).collect())
    }

    async fn get_signer(&self) -> Option<CelestiaAddress> {
        self.signer_address
    }

    async fn get_approximate_block_time(&self) -> Duration {
        Duration::from_secs(3)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unprovable_blob_gets_no_proof_without_affecting_others() {
        let data = bytes::Bytes::from((0..1_000u32).map(|i| (i * 7) as u8).collect::<Vec<_>>());
        let config = celestia_client::api::fibre::BlobConfig::for_version(0).unwrap();
        let commitment = celestia_client::api::fibre::EncodedFibreBlob::new(&data, config)
            .unwrap()
            .id()
            .commitment();
        let blob = |commitment| DownloadedFibreBlob {
            sender: CelestiaAddress::try_from([0xAA; 20].as_slice()).unwrap(),
            commitment,
            data: data.clone(),
        };
        let blobs = [blob([0x12; 32]), blob(commitment)];

        let proofs = prove_read_prefixes(&blobs, &[0, 0]);

        assert_eq!(
            proofs.iter().map(Option::is_some).collect::<Vec<_>>(),
            [false, true]
        );
    }
}
