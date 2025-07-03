use avail_rust::avail_core::data_proof::TxDataRoots;
use avail_rust::avail_core::from_substrate::blake2_256;
use avail_rust::avail_core::DataProof;
use avail_rust::error::ClientError;
use avail_rust::{
    AppId, Block, BlockNumber, Client, Filter, Keypair, Options, TransactionDetails, SDK,
};

use sov_rollup_interface::common::HexHash;
use sov_rollup_interface::da::{DaProof, DaSpec, RelevantBlobs, RelevantProofs, Time};
use sov_rollup_interface::node::da::{
    run_maybe_retryable_async_fn_with_retries, DaService, MaybeRetryable, SubmitBlobReceipt,
};

use backon::ExponentialBuilder;
use tokio::sync::oneshot;

use crate::types::blob::AvailDABlob;
use crate::types::block::AvailBlock;
use crate::types::config::AvailDAConfig;
use crate::types::data::AvailData;
use crate::types::error::AvailError;
use crate::types::header::AvailHeader;
use crate::verifier::{AvailDASpec, AvailDAVerifier};

type TimestampCall = avail_rust::avail::timestamp::calls::types::Set;

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
            .map(|tx_details| SubmitBlobReceipt {
                blob_hash: HexHash::from(blake2_256(blob)),
                da_transaction_id: tx_details.tx_hash.into(),
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
        .map(|tx_details| SubmitBlobReceipt {
            blob_hash: HexHash::from(blake2_256(aggregated_proof_data)),
            da_transaction_id: tx_details.tx_hash.into(),
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
                data_root: sp_core::H256::zero(),
                blob_root: sp_core::H256::zero(),
                bridge_root: sp_core::H256::zero(),
            },
            proof: vec![],
            number_of_leaves: 1,
            leaf_index: 0,
            leaf: sp_core::H256::zero(),
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
    async fn get_finalized_block_header(&self) -> Result<AvailHeader, AvailError> {
        let block = Block::new_finalized_block(&self.client)
            .await
            .map_err(AvailError)?;
        Ok(AvailHeader {
            header: block.block.header().clone(),
        })
    }

    async fn get_best_block_header(&self) -> Result<AvailHeader, AvailError> {
        let block = Block::new_best_block(&self.client)
            .await
            .map_err(AvailError)?;
        Ok(AvailHeader {
            header: block.block.header().clone(),
        })
    }

    async fn get_block_at_height(
        &self,
        block_number: BlockNumber,
    ) -> Result<AvailBlock, AvailError> {
        let block = Block::from_block_number(&self.client, block_number)
            .await
            .map_err(AvailError)?;

        // To find the block timestamp
        let block_transactions = block.transactions_static::<TimestampCall>(Filter::default());
        if block_transactions.len() != 1 {
            return Err(AvailError(ClientError::Custom(
                "Expected exactly 1 transaction".into(),
            )));
        }

        let filter = Filter::new();
        // To extract the blob transactions of batch submitted by batch_app_id
        let batch_blobs = block
            .data_submissions(filter.clone().app_id(self.batch_app_id.0 .0))
            .into_iter()
            .filter_map(|tx| {
                tx.account_id().map(|signer| AvailData {
                    data: tx.data,
                    signer,
                })
            })
            .collect();

        // To extract the blob transactions of proof submitted by proof_app_id
        let proof_blobs = block
            .data_submissions(filter.clone().app_id(self.proof_app_id.0 .0))
            .into_iter()
            .filter_map(|tx| {
                tx.account_id().map(|signer| AvailData {
                    data: tx.data,
                    signer,
                })
            })
            .collect();

        Ok(AvailBlock::new(
            crate::types::header::AvailHeader {
                header: block.block.header().clone(),
            },
            block.block.hash(),
            Time::from_secs(block_transactions[0].value.now as i64),
            batch_blobs,
            proof_blobs,
        ))
    }

    async fn submit_data(
        client: &Client,
        account: &Keypair,
        app_id: AppId,
        data: &[u8],
    ) -> Result<TransactionDetails, AvailError> {
        let sdk = SDK::new_custom(client.clone()).await.map_err(AvailError)?;
        let tx = sdk.tx.data_availability.submit_data(data.to_vec());
        let options = Options {
            app_id: Some(app_id.0 .0),
            ..Default::default()
        };
        let res = tx
            .execute_and_watch_finalization(&account, options)
            .await
            .map_err(AvailError)?;
        Ok(res)
    }
}
