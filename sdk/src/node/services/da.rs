use std::future::Future;

use crate::{
    da::DaSpec,
    serial::{Decode, Encode},
};

/// A DaService is the local side of an RPC connection talking to a node of the DA layer
/// It is *not* part of the logic that is zk-proven.
///
/// The DaService has two responsibilities - fetching data from the DA layer, transforming the
/// data into a representation that can be efficiently verified in circuit.
pub trait DaService {
    /// A handle to the types used by the DA layer.
    type Spec: DaSpec;

    /// A DA layer block, possibly excluding some irrelevant information.
    type FilteredBlock: SlotData;

    /// The output of an async call. Used in place of a dependency on async_trait.
    type Future<T>: Future<Output = Result<T, Self::Error>>;

    /// The error type for fallible methods.
    type Error: Send + Sync;

    /// Retrieve the data for the given height, waiting for it to be
    /// finalized if necessary. The block, once returned, must not be reverted
    /// without a consensus violation.
    fn get_finalized_at(&self, height: u64) -> Self::Future<Self::FilteredBlock>;

    /// Fetch the block at the given height, waiting for one to be mined if necessary.
    /// The returned block may not be final, and can be reverted without a consensus violation
    fn get_block_at(&self, height: u64) -> Self::Future<Self::FilteredBlock>;

    /// Extract the relevant transactions from a block. For example, this method might return
    /// all of the blob transactions in rollup's namespace on Celestia.
    fn extract_relevant_txs(
        &self,
        block: Self::FilteredBlock,
    ) -> Vec<<Self::Spec as DaSpec>::BlobTransaction>;

    /// Extract the relevant transactions from a block, along with a proof that the extraction has been done correctly.
    /// For example, this method might return all of the blob transactions in rollup's namespace on Celestia,
    /// together with a range proof against the root of the namespaced-merkle-tree, demonstrating that the entire
    /// rollup namespace has been covered.
    fn extract_relevant_txs_with_proof(
        &self,
        block: Self::FilteredBlock,
    ) -> (
        Vec<<Self::Spec as DaSpec>::BlobTransaction>,
        <Self::Spec as DaSpec>::InclusionMultiProof,
        <Self::Spec as DaSpec>::CompletenessProof,
    );

    // TODO: add a send_transaction method
    // fn send_transaction(tx: Self::Transaction, sender: Self::Address)
}

/// A trait for data from the DA layer that will be stored in the rollup's database.
pub trait SlotData: Encode + Decode + PartialEq + core::fmt::Debug + Clone {
    type BatchData;
    /// Encode any *non-batch* data (i.e. header, metadata, etc.) from this slot for storage. Batches contained
    /// in this slot are encoded and stored separately
    fn extra_data_for_storage(&self) -> Vec<u8>;
    fn reconstruct_from_storage(extra_data: &[u8], batches: Vec<Self::BatchData>) -> Self;
    fn hash(&self) -> [u8; 32];
}
