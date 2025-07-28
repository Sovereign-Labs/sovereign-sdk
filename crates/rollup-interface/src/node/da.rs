//! The da module defines traits used by the full node to interact with the DA layer.

use core::fmt::{Debug, Display};
use std::time::Duration;

use backon::{BackoffBuilder, Retryable};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;
use tracing::error;

use crate::common::HexHash;
use crate::da::{BlockHeaderTrait, DaSpec, DaVerifier, RelevantBlobs, RelevantProofs, Time};

/// Perform a checked arithmetic, returning None if the result is invalid.
pub trait CheckedMath<Rhs = Self> {
    /// The output type of arithmetic operations
    type Output;

    /// Performs checked multiplication, returning None if the result would overflow.
    fn checked_mul(&self, rhs: Rhs) -> Option<Self::Output>;
    /// Performs checked division, returning None if the caller attempts divide by zero or if the calculation overflows.
    fn checked_div(&self, rhs: Rhs) -> Option<Self::Output>;
    /// Performs checked subtraction, returning None if the result would underflow.
    fn checked_sub(&self, rhs: Rhs) -> Option<Self::Output>;
    /// Performs checked addition, returning None if the result would overflow.
    fn checked_add(&self, rhs: Rhs) -> Option<Self::Output>;
}

macro_rules! impl_checked_math_primitive {
    ($($t:ty),*) => {
        $(impl CheckedMath for $t {
            type Output = Self;

            fn checked_mul(&self, rhs: Self) -> Option<Self::Output> {
                <$t>::checked_mul(*self, rhs)
            }

            fn checked_div(&self, rhs: Self) -> Option<Self::Output> {
                <$t>::checked_div(*self, rhs)
            }

            fn checked_sub(&self, rhs: Self) -> Option<Self::Output> {
                <$t>::checked_sub(*self, rhs)
            }

            fn checked_add(&self, rhs: Self) -> Option<Self::Output> {
                <$t>::checked_add(*self, rhs)
            }
        })*
    };
}

impl_checked_math_primitive!(u8, u16, u32, u64, u128, usize);
impl_checked_math_primitive!(i8, i16, i32, i64, i128, isize);

/// The [`MaybeRetryable`] enum can be returned from a fallible function to
/// determine whether it can re-attempted or not.
#[derive(Debug, thiserror::Error)]
pub enum MaybeRetryable<E> {
    /// This error is a permanent one and thus the function that
    /// raised it should not be retried.
    #[error("{0}")]
    Permanent(E),
    /// This error is transient and thus the function that raised
    /// ought to be retried.
    #[error("{0}")]
    Transient(E),
}

impl<E: std::fmt::Display> MaybeRetryable<E> {
    fn is_retryable(&self) -> bool {
        matches!(self, Self::Transient(_))
    }

    fn into_err(self) -> E {
        match self {
            Self::Permanent(e) | Self::Transient(e) => e,
        }
    }
}

/// The default error for `MaybeRetryable` is assumed to be permanent
/// rather than something that can be retried.
impl<E> From<E> for MaybeRetryable<E> {
    fn from(e: E) -> MaybeRetryable<E> {
        Self::Permanent(e)
    }
}

/// Output of submit blob operation.
#[derive(Debug, Clone, Serialize, Deserialize, derive_more::Display)]
#[display(
    "SubmitBlobReceipt {{ blob_hash: {}, da_transaction_id: {:?} }}",
    blob_hash,
    da_transaction_id
)]
pub struct SubmitBlobReceipt<T: Debug + Clone> {
    /// Computed blob hash, so it can be identified by fetcher of the blobs.
    pub blob_hash: HexHash,
    /// Identifier of the transaction on the DA layer.
    pub da_transaction_id: T,
}

/// A DaService is the local side of an RPC connection talking to a node of the DA layer
/// It is *not* part of the logic that is zk-proven.
///
/// The DaService has two responsibilities - fetching data from the DA layer, transforming the
/// data into a representation that can be efficiently verified in circuit.
#[async_trait::async_trait]
pub trait DaService: Clone + Send + Sync + 'static {
    /// A handle to the types used by the DA layer.
    type Spec: DaSpec;

    /// [`serde`]-compatible configuration data for this [`DaService`]. Parsed
    /// from TOML.
    type Config: JsonSchema + PartialEq + Send + Sync + 'static;

    /// The verifier for this DA layer.
    type Verifier: DaVerifier<Spec = Self::Spec> + Clone;

    /// A DA layer block, possibly excluding some irrelevant information.
    type FilteredBlock: SlotData<BlockHeader = <Self::Spec as DaSpec>::BlockHeader>;

    /// The error type for fallible methods.
    type Error: Debug + Send + Sync + Display;

    /// Subsequent calls of [`DaService::send_transaction`] guarantee that the
    /// transactions are published and land in the DA layer in the same order as
    /// the method calls.
    const GUARANTEES_TRANSACTION_ORDERING: bool = false;

    /// Fetch the block at the given height, waiting for one to be mined if necessary.
    ///
    /// The returned block may not be final, and can be reverted without a consensus violation.
    /// Calls to this method for the same height are allowed to return different results.
    /// Should always returns the block at that height on the best fork.
    async fn get_block_at(&self, height: u64) -> Result<Self::FilteredBlock, Self::Error>;

    /// Similar to [`DaService::get_block_at`], but only returns the block header and not the whole block.
    /// All constraints and limitations of [`Self::get_block_at`] apply
    async fn get_block_header_at(
        &self,
        height: u64,
    ) -> Result<<Self::Spec as DaSpec>::BlockHeader, Self::Error> {
        let block = self.get_block_at(height).await?;
        Ok(block.header().clone())
    }

    /// How long the node should wait after a block is produced before
    /// submitting a transaction.
    ///
    /// The returned value must be low for the node to be reasonably confident
    /// that the transaction will be included in the next block.
    ///
    /// The [`DaService`] is allowed to adjust this value based on network
    /// conditions. Set to [`Duration::ZERO`] by default.
    fn safe_lead_time(&self) -> Duration {
        Duration::ZERO
    }

    /// Fetch the [`DaSpec::BlockHeader`] of the last finalized block.
    /// If there's no finalized block yet, it should return an error.
    async fn get_last_finalized_block_header(
        &self,
    ) -> Result<<Self::Spec as DaSpec>::BlockHeader, Self::Error>;

    /// Fetch the height of the last finalized block.
    ///
    /// ## Why not just use [`DaService::get_last_finalized_block_header`]?
    ///
    /// Some [`DaService`] implementations may have a way of querying for the
    /// latest finalized block number that's faster than fetching the whole
    /// header. This method allows for such [`DaService`]-specific
    /// optimizations.
    async fn get_last_finalized_block_number(&self) -> Result<u64, Self::Error> {
        Ok(self.get_last_finalized_block_header().await?.height())
    }

    /// Fetch the head block of the most popular fork.
    ///
    /// More like utility method, to provide better user experience
    async fn get_head_block_header(
        &self,
    ) -> Result<<Self::Spec as DaSpec>::BlockHeader, Self::Error>;

    /// Extract the relevant transactions from a block. For example, this method might return
    /// all of the blob transactions from a set rollup namespaces on Celestia.
    fn extract_relevant_blobs(
        &self,
        block: &Self::FilteredBlock,
    ) -> RelevantBlobs<<Self::Spec as DaSpec>::BlobTransaction>;

    /// Generate a proof that the relevant blob transactions have been extracted correctly from the DA layer
    /// block.
    async fn get_extraction_proof(
        &self,
        block: &Self::FilteredBlock,
        blobs: &RelevantBlobs<<Self::Spec as DaSpec>::BlobTransaction>,
    ) -> RelevantProofs<
        <Self::Spec as DaSpec>::InclusionMultiProof,
        <Self::Spec as DaSpec>::CompletenessProof,
    >;

    /// Extract the relevant transactions from a block, along with a proof that the extraction has been done correctly.
    /// For example, this method might return all of the blob transactions in rollup's namespace on Celestia,
    /// together with a range proof against the root of the namespaced-merkle-tree, demonstrating that the entire
    /// rollup namespace has been covered.
    #[allow(clippy::type_complexity)]
    async fn extract_relevant_blobs_with_proof(
        &self,
        block: &Self::FilteredBlock,
    ) -> (
        RelevantBlobs<<Self::Spec as DaSpec>::BlobTransaction>,
        RelevantProofs<
            <Self::Spec as DaSpec>::InclusionMultiProof,
            <Self::Spec as DaSpec>::CompletenessProof,
        >,
    ) {
        let relevant_blobs = self.extract_relevant_blobs(block);
        let relevant_proofs = self.get_extraction_proof(block, &relevant_blobs).await;
        (relevant_blobs, relevant_proofs)
    }

    /// Send a transaction directly to the DA layer.
    /// This method is infallible: the SubmitBlobReceipt is returned via the `oneshot::Receiver` after the blob is posted to the DA.
    async fn send_transaction(
        &self,
        blob: &[u8],
    ) -> oneshot::Receiver<
        Result<SubmitBlobReceipt<<Self::Spec as DaSpec>::TransactionId>, Self::Error>,
    >;

    /// Sends a proof to the DA layer.
    /// This method is infallible: the SubmitBlobReceipt is returned via the `oneshot::Receiver` after the blob is posted to the DA.
    async fn send_proof(
        &self,
        aggregated_proof_data: &[u8],
    ) -> oneshot::Receiver<
        Result<SubmitBlobReceipt<<Self::Spec as DaSpec>::TransactionId>, Self::Error>,
    >;

    /// Fetches all proofs at a specified block height.
    async fn get_proofs_at(&self, height: u64) -> Result<Vec<Vec<u8>>, Self::Error>;

    /// Returns a [`tokio::task::JoinHandle`] to the DA service background task,
    /// if it exists.
    async fn take_background_join_handle(&self) -> Option<tokio::task::JoinHandle<()>> {
        None
    }

    /// Returns a [`DaSpec::Address`] that signs blobs submitted by this instance of [`DaService`]
    async fn get_signer(&self) -> <Self::Spec as DaSpec>::Address;
}

/// Retry the given async function with the given backoff policy.
pub async fn run_maybe_retryable_async_fn_with_retries<F, Fut, T, E>(
    backoff_policy: impl BackoffBuilder,
    fxn: F,
    da_method_name: &str,
) -> Result<T, E>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<T, MaybeRetryable<E>>>,
    E: std::fmt::Display,
{
    fxn.retry(backoff_policy)
        .notify(|err: &MaybeRetryable<E>, dur: Duration| {
            tracing::warn!(
                method_name = da_method_name, error = %err, duration = ?dur,
                "Error in DA Service, will retry in specified duration."
            );
        })
        .when(MaybeRetryable::is_retryable)
        .await
        .map_err(MaybeRetryable::into_err)
}

/// `SlotData` is the subset of a DA layer block which is stored in the rollup's database.
/// At the very least, the rollup needs access to the hashes and headers of all DA layer blocks,
/// but rollup may choose to store partial (or full) block data as well.
pub trait SlotData:
    Serialize + DeserializeOwned + PartialEq + core::fmt::Debug + Clone + Send + Sync
{
    /// The header type for a DA layer block as viewed by the rollup. This need not be identical
    /// to the underlying rollup's header type, but it must be sufficient to reconstruct the block hash.
    ///
    /// For example, most fields of a Tendermint-based DA chain like Celestia are irrelevant to the rollup.
    /// For these fields, we only ever store their *serialized* representation in memory or on disk. Only a few special
    /// fields like `data_root` are stored in decoded form in the `CelestiaHeader` struct.
    type BlockHeader: BlockHeaderTrait;

    /// The canonical hash of the DA layer block.
    fn hash(&self) -> [u8; 32];
    /// The header of the DA layer block.
    fn header(&self) -> &Self::BlockHeader;
    /// The timestamp of the DA layer block.
    fn timestamp(&self) -> Time;
}

#[cfg(test)]
mod tests {
    use backon::ExponentialBuilder;

    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn should_run_async_fn_with_retries() {
        let error = "some error".to_string();
        let retry_counter = tokio::sync::Mutex::new(0);
        let max_retries = 3;
        let backoff_policy = ExponentialBuilder::default().with_max_times(max_retries);

        let r = run_maybe_retryable_async_fn_with_retries(
            backoff_policy,
            || async {
                let mut count = retry_counter.lock().await;
                *count += 1;
                Result::<(), MaybeRetryable<String>>::Err(MaybeRetryable::Transient(error.clone()))
            },
            "test_function",
        )
        .await;

        assert_eq!(r, Err(error));
        assert_eq!(*retry_counter.lock().await, max_retries + 1); // NOTE: Because first attempt is not a retry.
    }
}
