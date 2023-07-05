use std::fmt;
use std::future::Future;

use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::da::DaSpec;
use crate::traits::BlockHeaderTrait;

/// A DaService is the local side of an RPC connection talking to a node of the DA layer
/// It is *not* part of the logic that is zk-proven.
///
/// The DaService has two responsibilities - fetching data from the DA layer, transforming the
/// data into a representation that can be efficiently verified in circuit.
pub trait DaService {
    /// A handle to the types used by the DA layer.
    type RuntimeConfig: DeserializeOwned;

    /// A handle to the types used by the DA layer.
    type Spec: DaSpec;

    /// A DA layer block, possibly excluding some irrelevant information.
    type FilteredBlock: SlotData<BlockHeader = <Self::Spec as DaSpec>::BlockHeader>;

    /// The output of an async call. Used in place of a dependency on async_trait.
    type Future<T>: Future<Output = Result<T, Self::Error>> + Send;

    /// The error type for fallible methods.
    type Error: fmt::Debug + Send + Sync;

    /// Create a new instance of the DaService
    fn new(config: Self::RuntimeConfig, chain_params: <Self::Spec as DaSpec>::ChainParams) -> Self;

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
        block: &Self::FilteredBlock,
    ) -> Vec<<Self::Spec as DaSpec>::BlobTransaction>;

    fn get_extraction_proof(
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
    fn extract_relevant_txs_with_proof(
        &self,
        block: &Self::FilteredBlock,
    ) -> (
        Vec<<Self::Spec as DaSpec>::BlobTransaction>,
        <Self::Spec as DaSpec>::InclusionMultiProof,
        <Self::Spec as DaSpec>::CompletenessProof,
    ) {
        let relevant_txs = self.extract_relevant_txs(block);

        let (etx_proofs, rollup_row_proofs) =
            self.get_extraction_proof(block, relevant_txs.as_slice());

        (relevant_txs, etx_proofs, rollup_row_proofs)
    }

    /// Send a transaction directly to the DA layer.
    /// blob is the serialized and signed transaction.
    /// Returns nothing if the transaction was successfully sent.
    fn send_transaction(&self, blob: &[u8]) -> Self::Future<()>;
}

pub trait SlotData: Serialize + DeserializeOwned + PartialEq + core::fmt::Debug + Clone {
    type BlockHeader: BlockHeaderTrait;
    fn hash(&self) -> [u8; 32];
    fn header(&self) -> &Self::BlockHeader;
}
