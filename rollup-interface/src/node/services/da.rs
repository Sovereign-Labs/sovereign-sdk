//! The da module defines traits used by the full node to interact with the DA layer.
use std::fmt::{self, Display};

use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::da::{BlockHeaderTrait, DaSpec, DaVerifier};
use crate::zk::ValidityCondition;

/// A DaService is the local side of an RPC connection talking to a node of the DA layer
/// It is *not* part of the logic that is zk-proven.
///
/// The DaService has two responsibilities - fetching data from the DA layer, transforming the
/// data into a representation that can be efficiently verified in circuit.
#[async_trait]
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

    /// The error type for fallible methods.
    type Error: fmt::Debug + Send + Sync + Display;

    /// Retrieve the data for the given height, waiting for it to be
    /// finalized if necessary. The block, once returned, must not be reverted
    /// without a consensus violation.
    async fn get_finalized_at(&self, height: u64) -> Result<Self::FilteredBlock, Self::Error>;

    /// Fetch the block at the given height, waiting for one to be mined if necessary.
    /// The returned block may not be final, and can be reverted without a consensus violation
    async fn get_block_at(&self, height: u64) -> Result<Self::FilteredBlock, Self::Error>;

    /// Extract the relevant transactions from a block. For example, this method might return
    /// all of the blob transactions in rollup's namespace on Celestia.
    fn extract_relevant_txs(
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
    async fn extract_relevant_txs_with_proof(
        &self,
        block: &Self::FilteredBlock,
    ) -> (
        Vec<<Self::Spec as DaSpec>::BlobTransaction>,
        <Self::Spec as DaSpec>::InclusionMultiProof,
        <Self::Spec as DaSpec>::CompletenessProof,
    ) {
        let relevant_txs = self.extract_relevant_txs(block);

        let (etx_proofs, rollup_row_proofs) = self
            .get_extraction_proof(block, relevant_txs.as_slice())
            .await;

        (relevant_txs, etx_proofs, rollup_row_proofs)
    }

    /// Send a transaction directly to the DA layer.
    /// blob is the serialized and signed transaction.
    /// Returns nothing if the transaction was successfully sent.
    async fn send_transaction(&self, blob: &[u8]) -> Result<(), Self::Error>;
}

/// `SlotData` is the subset of a DA layer block which is stored in the rollup's database.
/// At the very least, the rollup needs access to the hashes and headers of all DA layer blocks, but rollups
/// may choose to partial (or full) block data as well.
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
