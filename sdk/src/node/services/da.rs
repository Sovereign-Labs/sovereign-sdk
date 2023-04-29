use std::future::Future;

use crate::{
    da::{DaSpec, VerifiableDaSpec},
    serial::{Decode, Encode},
};

/// A DaService is the local side of an RPC connection talking to a node of the DA layer
/// It is *not* part of the logic that is zk-proven.
pub trait DaService {
    /// A handle to the types used by the DA layer.
    type Spec: DaSpec;

    /// The output of an async call. Used in place of a dependency on async_trait.
    type Future<T>: Future<Output = Result<T, Self::Error>>;

    /// The error type for fallible methods.
    type Error: Send + Sync;

    /// Retrieve the data for the given height, waiting for it to be
    /// finalized if necessary. The block, once returned, must not be reverted
    /// without a consensus violation.
    fn get_finalized_at(&self, height: u64) -> Self::Future<<Self::Spec as DaSpec>::FilteredBlock>;

    /// Fetch the block at the given height, waiting for one to be mined if necessary.
    /// The returned block may not be final, and can be reverted without a consensus violation
    fn get_block_at(&self, height: u64) -> Self::Future<<Self::Spec as DaSpec>::FilteredBlock>;

    /// Extract the relevant transactions from a block. For example, this method might return
    /// all of the blob transactions in rollup's namespace on Celestia.
    fn extract_relevant_txs(
        &self,
        block: <Self::Spec as DaSpec>::FilteredBlock,
    ) -> Vec<<Self::Spec as DaSpec>::BlobTransaction>;
    // TODO: add a send_transaction method https://github.com/Sovereign-Labs/sovereign/issues/208
}

/// The VerifiableDaService extends a DaService with the ability to fetch proofs of data availability.
/// Like the DaService, it is *not* part of the logic that is zk-proven.
///
/// The service should transform its output data into a representation that can be efficiently verified in circuit.
pub trait VerifiableDaService: DaService {
    type Spec: VerifiableDaSpec;

    /// Extract the relevant transactions from a block, along with a proof that the extraction has been done correctly.
    /// For example, this method might return all of the blob transactions in rollup's namespace on Celestia,
    /// together with a range proof against the root of the namespaced-merkle-tree, demonstrating that the entire
    /// rollup namespace has been covered.
    fn extract_relevant_txs_with_proof(
        &self,
        block: <<Self as VerifiableDaService>::Spec as VerifiableDaSpec>::FilteredBlock,
    ) -> (
        Vec<<<Self as VerifiableDaService>::Spec as VerifiableDaSpec>::BlobTransaction>,
        <<Self as VerifiableDaService>::Spec as VerifiableDaSpec>::InclusionMultiProof,
        <<Self as VerifiableDaService>::Spec as VerifiableDaSpec>::CompletenessProof,
    );
}

pub trait SlotData: Encode + Decode + PartialEq + core::fmt::Debug + Clone {
    fn hash(&self) -> [u8; 32];
}
