use avail_rust_client::avail::data_availability::tx::SubmitData;
use avail_rust_client::avail_rust_core::rpc::kate::{DataProof, TxDataRoots};
use avail_rust_client::avail_rust_core::rpc::system::fetch_extrinsics_v1_types::{
    EncodeSelector, SignatureFilter,
};
use avail_rust_client::avail_rust_core::AppId;
use avail_rust_client::{
    AccountId, AccountIdExt, Client, HashNumber, Keypair, KeypairExt, Options,
    TransactionDecodable, H256,
};

use sov_rollup_interface::common::HexHash;
use sov_rollup_interface::da::{DaProof, DaSpec, RelevantBlobs, RelevantProofs, Time};
use sov_rollup_interface::node::da::{
    run_maybe_retryable_async_fn_with_retries, DaService, MaybeRetryable, SubmitBlobReceipt,
};

use backon::ExponentialBuilder;
use sp_core::blake2_256;
use tokio::sync::oneshot;

use crate::types::blob::AvailDABlob;
use crate::types::block::AvailBlock;
use crate::types::config::AvailDAConfig;
use crate::types::data::AvailData;
use crate::types::header::CustomAvailHeader;
use crate::types::utils::CustomTransaction;
use crate::verifier::{AvailDASpec, AvailDAVerifier};
use tracing::{debug, error, info, instrument, trace, warn};

#[derive(Clone)]
pub struct AvailDAService {
    pub client: Client,
    pub proof_app_id: AppId,
    pub batch_app_id: AppId,
    pub signer: Keypair,
    backoff_policy: ExponentialBuilder,
}

#[async_trait::async_trait]
impl DaService for AvailDAService {
    type Spec = AvailDASpec;
    type Config = AvailDAConfig;
    type Verifier = AvailDAVerifier;
    type FilteredBlock = AvailBlock;
    type Error = anyhow::Error;

    #[instrument(skip(self))]
    async fn get_block_at(&self, height: u64) -> Result<Self::FilteredBlock, Self::Error> {
        info!("Attempting to get block at height: {}", height);
        run_maybe_retryable_async_fn_with_retries(
            &self.backoff_policy,
            || async {
                self.get_block_at_height(height.try_into().unwrap())
                    .await
                    .map_err(MaybeRetryable::Transient)
            },
            "get_block_at",
        )
        .await
    }

    #[instrument(skip(self))]
    async fn get_last_finalized_block_header(
        &self,
    ) -> Result<<Self::Spec as DaSpec>::BlockHeader, Self::Error> {
        info!("Attempting to get last finalized block header");
        run_maybe_retryable_async_fn_with_retries(
            &self.backoff_policy,
            || async {
                self.get_finalized_block_header()
                    .await
                    .map_err(MaybeRetryable::Transient)
            },
            "get_last_finalised_block_header",
        )
        .await
    }

    #[instrument(skip(self))]
    async fn get_head_block_header(
        &self,
    ) -> Result<<Self::Spec as DaSpec>::BlockHeader, Self::Error> {
        info!("Attempting to get head block header");
        run_maybe_retryable_async_fn_with_retries(
            &self.backoff_policy,
            || async {
                self.get_best_block_header()
                    .await
                    .map_err(MaybeRetryable::Transient)
            },
            "get_head_block_header",
        )
        .await
    }

    #[instrument(skip(self, block))]
    fn extract_relevant_blobs(
        &self,
        block: &Self::FilteredBlock,
    ) -> RelevantBlobs<<Self::Spec as DaSpec>::BlobTransaction> {
        debug!(
            "Extracting relevant blobs from block number: {}",
            block.header.header.number
        );
        let batch_blobs: Vec<AvailDABlob> = block
            .batch_blobs
            .iter()
            .cloned()
            .map(AvailDABlob::from)
            .collect();
        debug!("Extracted {} batch blobs", batch_blobs.len());

        let proof_blobs: Vec<AvailDABlob> = block
            .proof_blobs
            .iter()
            .cloned()
            .map(AvailDABlob::from)
            .collect();
        debug!("Extracted {} proof blobs", proof_blobs.len());

        RelevantBlobs {
            batch_blobs,
            proof_blobs,
        }
    }

    #[instrument(skip(self, blob))]
    async fn send_transaction(
        &self,
        blob: &[u8],
    ) -> oneshot::Receiver<
        Result<SubmitBlobReceipt<<Self::Spec as DaSpec>::TransactionId>, Self::Error>,
    > {
        info!(
            "Sending transaction with blob of size: {} bytes",
            blob.len()
        );
        let (tx, rx) = oneshot::channel();
        let result = Self::submit_data(&self.client, &self.signer, self.batch_app_id, blob)
            .await
            .map(|tx_hash| SubmitBlobReceipt {
                blob_hash: HexHash::from(blake2_256(blob)),
                da_transaction_id: tx_hash,
            });
        debug!("Transaction submission result: {:?}", result.is_ok());
        let _ = tx.send(result);
        rx
    }

    #[instrument(skip(self, aggregated_proof_data))]
    async fn send_proof(
        &self,
        aggregated_proof_data: &[u8],
    ) -> oneshot::Receiver<
        Result<SubmitBlobReceipt<<Self::Spec as DaSpec>::TransactionId>, Self::Error>,
    > {
        info!(
            "Sending proof with data of size: {} bytes",
            aggregated_proof_data.len()
        );
        let (tx, rx) = oneshot::channel();
        let result = Self::submit_data(
            &self.client,
            &self.signer,
            self.proof_app_id,
            aggregated_proof_data,
        )
        .await
        .map(|tx_hash: H256| SubmitBlobReceipt {
            blob_hash: HexHash::from(blake2_256(aggregated_proof_data)),
            da_transaction_id: tx_hash,
        });
        debug!("Proof submission result: {:?}", result.is_ok());
        let _ = tx.send(result);
        rx
    }

    #[instrument(skip(self))]
    async fn get_proofs_at(&self, height: u64) -> Result<Vec<Vec<u8>>, Self::Error> {
        warn!(
            "get_proofs_at method is not implemented yet for height: {}",
            height
        );
        Err(anyhow::anyhow!(
            "get_proofs_at method is not implemented yet".to_string(),
        ))
    }

    #[instrument(skip(self, block, blobs))]
    async fn get_extraction_proof(
        &self,
        block: &Self::FilteredBlock,
        blobs: &RelevantBlobs<<Self::Spec as DaSpec>::BlobTransaction>,
    ) -> RelevantProofs<
        <Self::Spec as DaSpec>::InclusionMultiProof,
        <Self::Spec as DaSpec>::CompletenessProof,
    > {
        debug!(
            "Getting extraction proof for block number: {}",
            block.header.header.number
        );
        let dummy_proof = DataProof {
            roots: TxDataRoots {
                data_root: H256::zero(),
                blob_root: H256::zero(),
                bridge_root: H256::zero(),
            },
            proof: vec![],
            number_of_leaves: 1,
            leaf_index: 0,
            leaf: H256::zero(),
        };
        trace!("Created dummy proof: {:?}", dummy_proof);

        RelevantProofs {
            batch: DaProof {
                inclusion_proof: dummy_proof.clone(),
                completeness_proof: dummy_proof.clone(),
            },
            proof: DaProof {
                inclusion_proof: dummy_proof.clone(),
                completeness_proof: dummy_proof.clone(),
            },
        }
    }
}

impl AvailDAService {
    #[instrument(skip(config))]
    pub async fn new_from_config(config: AvailDAConfig) -> Result<Self, anyhow::Error> {
        info!("Creating new AvailDAService from config");
        let client = Client::new(&config.http_api_url).await?;
        debug!("Avail client created for URL: {}", config.http_api_url);

        let account = Keypair::from_str(&config.signer_key).map_err(|err| {
            error!("Failed to create keypair from signer key: {}", err);
            anyhow::anyhow!(err)
        })?;
        debug!("Keypair created successfully");

        // NOTE: Current exponential backoff policy defaults:
        // jitter: false, factor: 2, min_delay: 1s, max_delay: 60s, max_times: 3,
        let backoff_policy = ExponentialBuilder::default();
        debug!("Backoff policy initialized");

        info!("AvailDAService created successfully");
        Ok(AvailDAService {
            client,
            proof_app_id: AppId(config.proof_app_id),
            batch_app_id: AppId(config.batch_app_id),
            signer: account,
            backoff_policy,
        })
    }

    #[instrument(skip(self))]
    async fn get_finalized_block_header(&self) -> Result<CustomAvailHeader, anyhow::Error> {
        info!("Fetching finalized block header");
        let header = self.client.finalized_block_header().await?;
        debug!("Finalized block header retrieved: {:?}", header.number);
        Ok(CustomAvailHeader { header })
    }

    #[instrument(skip(self))]
    async fn get_best_block_header(&self) -> Result<CustomAvailHeader, anyhow::Error> {
        info!("Fetching best block header");
        let header = self.client.best_block_header().await?;
        debug!("Best block header retrieved: {:?}", header.number);
        Ok(CustomAvailHeader { header })
    }

    #[instrument(skip(self))]
    async fn get_block_at_height(&self, block_number: u32) -> Result<AvailBlock, anyhow::Error> {
        info!("Fetching block at height: {}", block_number);
        let block_hash = self.client.block_hash(block_number).await?.ok_or_else(|| {
            anyhow::anyhow!("Block hash not found for block number {}", block_number)
        })?;
        debug!("Retrieved block hash: {:?}", block_hash);

        let block_client = self.client.block_client();

        let block_header = self.client.block_header(block_hash).await?.ok_or_else(|| {
            anyhow::anyhow!("Block header not found for block hash {:?}", block_hash)
        })?;
        debug!("Retrieved block header: {:?}", block_header);

        info!("Fetching batch blobs for block {}", block_number);
        let batch_blobs: Vec<avail_rust_client::avail_rust_core::rpc::system::fetch_extrinsics_v1_types::ExtrinsicInformation> = block_client
            .block_transactions(HashNumber::Number(block_number), None, Some(SignatureFilter::new(None, Some(self.batch_app_id.0), None)),Some(EncodeSelector::Call)).await?;
        debug!("Found {} batch blobs", batch_blobs.len());

        let processed_batch_blobs: Vec<AvailData> = batch_blobs
            .iter()
            .filter_map(|item| {
                let encoded_str = item.encoded.as_ref().map(|s| s.as_str());
                let submit_data = encoded_str.and_then(|s| SubmitData::decode_hex_call(s));
                if let Some(submit_data) = submit_data {
                    debug!(
                        "Processing batch blob with data size: {}",
                        submit_data.data.len()
                    );
                    let signer = item
                        .signature
                        .as_ref()
                        .and_then(|sig| sig.ss58_address.as_ref())
                        .and_then(|addr| AccountId::from_str(addr).ok());
                    if let Some(signer) = signer {
                        Some(AvailData {
                            data: submit_data.data,
                            signer,
                        })
                    } else {
                        warn!("Signer address not found or invalid for batch blob");
                        None
                    }
                } else {
                    warn!("Failed to decode submit data for batch blob");
                    None
                }
            })
            .collect();
        info!("Processed {} batch blobs", processed_batch_blobs.len());

        info!("Fetching proof blobs for block {}", block_number);
        let proof_blobs: Vec<avail_rust_client::avail_rust_core::rpc::system::fetch_extrinsics_v1_types::ExtrinsicInformation> = block_client
        .block_transactions(HashNumber::Number(block_number), None, Some(SignatureFilter::new(None, Some(self.proof_app_id.0), None)),Some(EncodeSelector::Call)).await?;
        debug!("Found {} proof blobs", proof_blobs.len());

        let processed_proof_blobs: Vec<AvailData> = proof_blobs
            .iter()
            .filter_map(|item| {
                let encoded_str = item.encoded.as_ref().map(|s| s.as_str());
                let submit_data = encoded_str.and_then(|s| SubmitData::decode_hex_call(s));
                if let Some(submit_data) = submit_data {
                    debug!(
                        "Processing proof blob with data size: {}",
                        submit_data.data.len()
                    );
                    let signer = item
                        .signature
                        .as_ref()
                        .and_then(|sig| sig.ss58_address.as_ref())
                        .and_then(|addr| AccountId::from_str(addr).ok());
                    if let Some(signer) = signer {
                        Some(AvailData {
                            data: submit_data.data,
                            signer,
                        })
                    } else {
                        warn!("Signer address not found or invalid for proof blob");
                        None
                    }
                } else {
                    warn!("Failed to decode submit data for proof blob");
                    None
                }
            })
            .collect();
        info!("Processed {} proof blobs", processed_proof_blobs.len());

        let tx = block_client
            .block_transaction(
                block_hash.into(),
                0.into(),
                None,
                Some(EncodeSelector::Call),
            )
            .await?
            .ok_or_else(|| {
                anyhow::anyhow!("Transaction not found for block hash {:?}", block_hash)
            })?;
        debug!("Retrieved transaction: {:?}", tx);

        let encoded_call_str = tx
            .encoded
            .ok_or_else(|| anyhow::anyhow!("Encoded call not found in transaction"))?
            .to_string();
        debug!("Decoded transaction call string: {}", encoded_call_str);

        let custom_tx = CustomTransaction::decode_hex_call(&encoded_call_str)
            .ok_or_else(|| anyhow::anyhow!("Failed to decode custom transaction"))?;
        debug!("Custom transaction: {:?}", custom_tx);

        info!("Successfully processed block at height {}", block_number);
        Ok(AvailBlock::new(
            CustomAvailHeader {
                header: block_header,
            },
            block_hash,
            Time::from_secs(custom_tx.set as i64),
            processed_batch_blobs,
            processed_proof_blobs,
        ))
    }

    #[instrument(skip(client, account, data))]
    async fn submit_data(
        client: &Client,
        account: &Keypair,
        app_id: AppId,
        data: &[u8],
    ) -> Result<H256, anyhow::Error> {
        info!(
            "Submitting data of size {} bytes for app_id {}",
            data.len(),
            app_id.0
        );
        let submittable_tx = client.tx().data_availability().submit_data(data.to_vec());
        debug!("Submittable transaction created");
        let submitted_tx = submittable_tx
            .sign_and_submit(account, Options::new(Some(app_id.0)))
            .await?;
        let res = submitted_tx.tx_hash;
        info!("Data submitted successfully, transaction hash: {:?}", res);
        Ok(res)
    }
}
