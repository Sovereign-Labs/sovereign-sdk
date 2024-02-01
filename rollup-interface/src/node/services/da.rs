//! The da module defines traits used by the full node to interact with the DA layer.

use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::da::BlockHeaderTrait;
#[cfg(feature = "native")]
use crate::da::{DaSpec, DaVerifier};
#[cfg(feature = "native")]
use crate::maybestd::vec::Vec;
use crate::zk::ValidityCondition;

/// A DaService is the local side of an RPC connection talking to a node of the DA layer
/// It is *not* part of the logic that is zk-proven.
///
/// The DaService has two responsibilities - fetching data from the DA layer, transforming the
/// data into a representation that can be efficiently verified in circuit.
#[cfg(feature = "native")]
#[async_trait::async_trait]
pub trait DaService: Send + Sync + 'static {
    /// A handle to the types used by the DA layer.
    type Spec: DaSpec;

    /// The verifier for this DA layer.
    type Verifier: DaVerifier<Spec = Self::Spec>;

    /// A DA layer block, possibly excluding some irrelevant information.
    type FilteredBlock: SlotData<
        BlockHeader = <Self::Spec as DaSpec>::BlockHeader,
        Cond = <Self::Spec as DaSpec>::ValidityCondition,
    >;

    /// Type that allow to consume [`futures::Stream`] of BlockHeaders.
    type HeaderStream: futures::Stream<
        Item = Result<<Self::Spec as DaSpec>::BlockHeader, Self::Error>,
    >;

    /// A transaction ID, used to identify the transaction in the DA layer.
    type TransactionId: PartialEq + Eq + PartialOrd + Ord + core::hash::Hash;

    /// The error type for fallible methods.
    type Error: core::fmt::Debug + Send + Sync + core::fmt::Display;

    /// Fetch the block at the given height, waiting for one to be mined if necessary.
    /// The returned block may not be final, and can be reverted without a consensus violation.
    /// Call it for the same height are allowed to return different results.
    /// Should always returns the block at that height on the best fork.
    async fn get_block_at(&self, height: u64) -> Result<Self::FilteredBlock, Self::Error>;

    /// Fetch the [`DaSpec::BlockHeader`] of the last finalized block.
    /// If there's no finalized block yet, it should return an error.
    async fn get_last_finalized_block_header(
        &self,
    ) -> Result<<Self::Spec as DaSpec>::BlockHeader, Self::Error>;

    /// Subscribe to finalized headers as they are finalized.
    /// Expect only to receive headers which were finalized after subscription
    /// Optimized version of `get_last_finalized_block_header`.
    async fn subscribe_finalized_header(&self) -> Result<Self::HeaderStream, Self::Error>;

    /// Fetch the head block of the most popular fork.
    ///
    /// More like utility method, to provide better user experience
    async fn get_head_block_header(
        &self,
    ) -> Result<<Self::Spec as DaSpec>::BlockHeader, Self::Error>;

    /// Extract the relevant transactions from a block. For example, this method might return
    /// all of the blob transactions in rollup's namespace on Celestia.
    fn extract_relevant_blobs(
        &self,
        block: &Self::FilteredBlock,
    ) -> Vec<<Self::Spec as DaSpec>::BlobTransaction>;

    /// Generate a proof that the relevant blob transactions have been extracted correctly from the DA layer
    /// block.
    async fn get_extraction_proof(
        &self,
        block: &Self::FilteredBlock,
        blobs: &[<Self::Spec as DaSpec>::BlobTransaction],
    ) -> (
        <Self::Spec as DaSpec>::InclusionMultiProof,
        <Self::Spec as DaSpec>::CompletenessProof,
    );

    /// Extract the relevant transactions from a block, along with a proof that the extraction has been done correctly.
    /// For example, this method might return all of the blob transactions in rollup's namespace on Celestia,
    /// together with a range proof against the root of the namespaced-merkle-tree, demonstrating that the entire
    /// rollup namespace has been covered.
    #[allow(clippy::type_complexity)]
    async fn extract_relevant_blobs_with_proof(
        &self,
        block: &Self::FilteredBlock,
    ) -> (
        Vec<<Self::Spec as DaSpec>::BlobTransaction>,
        <Self::Spec as DaSpec>::InclusionMultiProof,
        <Self::Spec as DaSpec>::CompletenessProof,
    ) {
        let relevant_txs = self.extract_relevant_blobs(block);

        let (etx_proofs, rollup_row_proofs) = self
            .get_extraction_proof(block, relevant_txs.as_slice())
            .await;

        (relevant_txs, etx_proofs, rollup_row_proofs)
    }

    /// Send a transaction directly to the DA layer.
    /// blob is the serialized and signed transaction.
    /// Returns nothing if the transaction was successfully sent.
    async fn send_transaction(&self, blob: &[u8]) -> Result<Self::TransactionId, Self::Error>;

    /// Sends am aggregated ZK proofs to the DA layer.
    async fn send_aggregated_zk_proof(
        &self,
        aggregated_proof_data: &[u8],
    ) -> Result<u64, Self::Error>;

    /// Fetches all aggregated ZK proofs at a specified block height.
    async fn get_aggregated_proofs_at(&self, height: u64) -> Result<Vec<Vec<u8>>, Self::Error>;
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
    /// For example, most fields of the a Tendermint-based DA chain like Celestia are irrelevant to the rollup.
    /// For these fields, we only ever store their *serialized* representation in memory or on disk. Only a few special
    /// fields like `data_root` are stored in decoded form in the `CelestiaHeader` struct.
    type BlockHeader: BlockHeaderTrait;

    /// The validity condition associated with the slot data.
    type Cond: ValidityCondition;

    /// The canonical hash of the DA layer block.
    fn hash(&self) -> [u8; 32];
    /// The header of the DA layer block.
    fn header(&self) -> &Self::BlockHeader;
    /// Get the validity condition set associated with the slot
    fn validity_condition(&self) -> Self::Cond;
}
