use bytes::Buf;

use crate::core::traits::{AddressTrait, BlockHeaderTrait};
use crate::serial::{Decode, DeserializationError, Encode};
use crate::services::da::SlotData;
use core::fmt::Debug;

/// A specification for the types used by a DA layer.
pub trait DaSpec {
    /// The hash of a DA layer block
    type SlotHash: BlockHashTrait;

    /// The address type used by the DA layer
    type Address: AddressTrait;

    /// The block header type used by the DA layer
    type BlockHeader: BlockHeaderTrait<Hash = Self::SlotHash>;

    /// The transaction type used by the DA layer.
    type BlobTransaction: BlobTransactionTrait<Self::Address>;

    /// A DA layer block, possibly excluding some irrelevant information.
    type FilteredBlock: SlotData;
}

pub trait VerifiableDaSpec:
    DaSpec<
    SlotHash = <Self as VerifiableDaSpec>::SlotHash,
    Address = <Self as VerifiableDaSpec>::Address,
    BlockHeader = <Self as VerifiableDaSpec>::BlockHeader,
    BlobTransaction = <Self as VerifiableDaSpec>::BlobTransaction,
    FilteredBlock = <Self as VerifiableDaSpec>::FilteredBlock,
>
{
    /// The hash of a DA layer block
    type SlotHash: BlockHashTrait;

    /// The address type used by the DA layer
    type Address: AddressTrait;

    /// The block header type used by the DA layer
    type BlockHeader: BlockHeaderTrait<Hash = <Self as VerifiableDaSpec>::SlotHash>;

    /// The transaction type used by the DA layer.
    type BlobTransaction: BlobTransactionTrait<<Self as VerifiableDaSpec>::Address>;

    /// A DA layer block, possibly excluding some irrelevant information.
    type FilteredBlock: SlotData;

    /// A proof that each tx in a set of blob transactions is included in a given block.
    type InclusionMultiProof: Encode + Decode;

    /// A proof that a claimed set of transactions is complete. For example, this could be a range
    /// proof demonstrating that the provided BlobTransactions represent the entire contents of Celestia namespace
    /// in a given block
    type CompletenessProof: Encode + Decode;
}

/// A ZkDaVerifier implements the logic required to create a zk proof that some data
/// has been processed.
///
/// This trait implements the required functionality to *verify* claims of the form
/// "If X is the most recent block in the DA layer, then Y is the ordered set of transactions that must
/// be processed by the rollup."
pub trait ZkDaVerifier {
    /// The set of types required by the DA layer.
    type Spec: VerifiableDaSpec;

    /// The error type returned by the DA layer's verification function
    type Error: Debug;

    /// Verify a claimed set of transactions against a block header.
    fn verify_relevant_tx_list(
        &self,
        block_header: &<Self::Spec as VerifiableDaSpec>::BlockHeader,
        txs: &[<Self::Spec as VerifiableDaSpec>::BlobTransaction],
        inclusion_proof: <Self::Spec as VerifiableDaSpec>::InclusionMultiProof,
        completeness_proof: <Self::Spec as VerifiableDaSpec>::CompletenessProof,
    ) -> Result<(), Self::Error>;
}

/// An OutOfBandDaVerifier verifies that a claimed set of transactions is complete and correct by
/// asking a trusted client of the DA layer. This trusted client *should* be run locally, but that
/// is not enforced by this trait
pub trait OutOfBandDaVerifier {
    /// The set of types required by the DA layer.
    type Spec: DaSpec;

    /// The error type returned by the DA layer's verification function
    type Error: Debug;

    type Future;

    /// Ask a trusted light client to verify that a claimed set of transactions is complete and correct.
    fn verify_relevant_tx_list(
        &self,
        block_hash: &<Self::Spec as DaSpec>::BlockHeader,
        txs: &[<Self::Spec as DaSpec>::BlobTransaction],
    ) -> Result<(), Self::Error>;
}

/// A transaction on a data availability layer, including the address of the sender.
pub trait BlobTransactionTrait<Addr>: Encode + Decode {
    type Data: Buf;
    /// Returns the address (on the DA layer) of the entity which submitted the blob transaction
    fn sender(&self) -> Addr;
    /// The raw data of the blob. For example, the "calldata" of an Ethereum rollup transaction
    fn data(&self) -> Self::Data;
}

pub trait BlockHashTrait:
    Encode + Decode<Error = DeserializationError> + PartialEq + Debug + Send + Sync
{
}
