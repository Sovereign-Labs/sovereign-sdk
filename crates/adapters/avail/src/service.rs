use avail_rust_client::avail::data_availability::tx::SubmitData;
use avail_rust_client::avail_rust_core::rpc::kate::{DataProof, TxDataRoots};
use avail_rust_client::avail_rust_core::rpc::system::fetch_extrinsics_v1_types::{
    EncodeSelector, SignatureFilter,
};
use avail_rust_client::avail_rust_core::AppId;
use avail_rust_client::error::ClientError;
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
use crate::types::error::AvailError;
use crate::types::header::CustomAvailHeader;
use crate::types::utils::CustomTransaction;
use crate::verifier::{AvailDASpec, AvailDAVerifier};

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
    type Error = AvailError;

    async fn get_block_at(&self, height: u64) -> Result<Self::FilteredBlock, Self::Error> {
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

    async fn get_last_finalized_block_header(
        &self,
    ) -> Result<<Self::Spec as DaSpec>::BlockHeader, Self::Error> {
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

    async fn get_head_block_header(
        &self,
    ) -> Result<<Self::Spec as DaSpec>::BlockHeader, Self::Error> {
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

    fn extract_relevant_blobs(
        &self,
        block: &Self::FilteredBlock,
    ) -> RelevantBlobs<<Self::Spec as DaSpec>::BlobTransaction> {
        let batch_blobs = block
            .batch_blobs
            .iter()
            .cloned()
            .map(AvailDABlob::from)
            .collect();

        let proof_blobs = block
            .proof_blobs
            .iter()
            .cloned()
            .map(AvailDABlob::from)
            .collect();

        RelevantBlobs {
            batch_blobs,
            proof_blobs,
        }
    }

    async fn send_transaction(
        &self,
        blob: &[u8],
    ) -> oneshot::Receiver<
        Result<SubmitBlobReceipt<<Self::Spec as DaSpec>::TransactionId>, Self::Error>,
    > {
        let (tx, rx) = oneshot::channel();
        let result = Self::submit_data(&self.client, &self.signer, self.batch_app_id, blob)
            .await
            .map(|tx_hash| SubmitBlobReceipt {
                blob_hash: HexHash::from(blake2_256(blob)),
                da_transaction_id: tx_hash,
            });
        let _ = tx.send(result);
        rx
    }

    async fn send_proof(
        &self,
        aggregated_proof_data: &[u8],
    ) -> oneshot::Receiver<
        Result<SubmitBlobReceipt<<Self::Spec as DaSpec>::TransactionId>, Self::Error>,
    > {
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
        let _ = tx.send(result);
        rx
    }

    async fn get_proofs_at(&self, height: u64) -> Result<Vec<Vec<u8>>, Self::Error> {
        Err(AvailError(ClientError::Custom(
            "get_proofs_at method is not implemented yet".to_string(),
        )))
    }
    async fn get_extraction_proof(
        &self,
        block: &Self::FilteredBlock,
        blobs: &RelevantBlobs<<Self::Spec as DaSpec>::BlobTransaction>,
    ) -> RelevantProofs<
        <Self::Spec as DaSpec>::InclusionMultiProof,
        <Self::Spec as DaSpec>::CompletenessProof,
    > {
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
    pub async fn new_from_config(config: AvailDAConfig) -> Result<Self, AvailError> {
        let client = Client::new(&config.http_api_url).await?;

        let account = Keypair::from_str(&config.signer_key)
            .map_err(|err| AvailError(ClientError::Custom(err)))?;

        // NOTE: Current exponential backoff policy defaults:
        // jitter: false, factor: 2, min_delay: 1s, max_delay: 60s, max_times: 3,
        let backoff_policy = ExponentialBuilder::default();

        Ok(AvailDAService {
            client,
            proof_app_id: AppId(config.proof_app_id),
            batch_app_id: AppId(config.batch_app_id),
            signer: account,
            backoff_policy,
        })
    }

    async fn get_finalized_block_header(&self) -> Result<CustomAvailHeader, AvailError> {
        let header = self.client.finalized_block_header().await?;
        Ok(CustomAvailHeader { header })
    }

    async fn get_best_block_header(&self) -> Result<CustomAvailHeader, AvailError> {
        let header = self.client.best_block_header().await?;
        Ok(CustomAvailHeader { header })
    }

    async fn get_block_at_height(&self, block_number: u32) -> Result<AvailBlock, AvailError> {
        let block_hash = self
            .client
            .block_hash(block_number)
            .await
            .map_err(|e| AvailError(ClientError::Core(e)))?;

        let block_client = self.client.block_client();

        let block_header = self.client.block_header(block_hash.unwrap()).await?;

        let batch_blobs: Vec<avail_rust_client::avail_rust_core::rpc::system::fetch_extrinsics_v1_types::ExtrinsicInformation> = block_client
            .block_transactions(HashNumber::Number(block_number), None, Some(SignatureFilter::new(None, Some(self.batch_app_id.0), None)),Some(EncodeSelector::Call)).await?;

        let batch_blobs: Vec<AvailData> = batch_blobs
            .iter()
            .map(|item| {
                let submit_data =
                    SubmitData::decode_hex_call(item.encoded.as_ref().unwrap().as_str());
                AvailData {
                    data: submit_data.unwrap().data,
                    signer: AccountId::from_str(
                        item.signature
                            .as_ref()
                            .unwrap()
                            .ss58_address
                            .as_ref()
                            .unwrap()
                            .as_str(),
                    )
                    .unwrap(),
                }
            })
            .collect();

        let proof_blobs: Vec<avail_rust_client::avail_rust_core::rpc::system::fetch_extrinsics_v1_types::ExtrinsicInformation> = block_client
        .block_transactions(HashNumber::Number(block_number), None, Some(SignatureFilter::new(None, Some(self.proof_app_id.0), None)),Some(EncodeSelector::Call)).await?;

        let proof_blobs: Vec<AvailData> = proof_blobs
            .iter()
            .map(|item| {
                let submit_data =
                    SubmitData::decode_hex_call(item.encoded.as_ref().unwrap().as_str());
                AvailData {
                    data: submit_data.unwrap().data,
                    signer: AccountId::from_str(
                        item.signature
                            .as_ref()
                            .unwrap()
                            .ss58_address
                            .as_ref()
                            .unwrap()
                            .as_str(),
                    )
                    .unwrap(),
                }
            })
            .collect();

        let tx = block_client
            .block_transaction(block_hash.unwrap().into(), 0.into(), None, None)
            .await?;
        let encoded_call = hex::decode(tx.unwrap().encoded.unwrap().trim_start_matches("0x"));
        let custom_tx = CustomTransaction::decode_call(&encoded_call.unwrap());

        Ok(AvailBlock::new(
            CustomAvailHeader {
                header: block_header.unwrap(),
            },
            block_hash.unwrap(),
            Time::from_secs(custom_tx.unwrap().set as i64),
            batch_blobs,
            proof_blobs,
        ))
    }

    async fn submit_data(
        client: &Client,
        account: &Keypair,
        app_id: AppId,
        data: &[u8],
    ) -> Result<H256, AvailError> {
        let submittable_tx = client.tx().data_availability().submit_data(data.to_vec());
        let submitted_tx = submittable_tx
            .sign_and_submit(account, Options::new(Some(app_id.0)))
            .await
            .map_err(|e| AvailError(ClientError::Core(e)))?;
        let res = submitted_tx.tx_hash;
        Ok(res)
    }
}
