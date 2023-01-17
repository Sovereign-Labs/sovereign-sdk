use bytes::Buf;

use crate::{
    core::traits::{self, BlockheaderTrait, AddressTrait},
    serial::{Deser, Serialize},
};
use core::fmt::Debug;

/// A DaApp implements the logic required to create a zk proof that some data
/// has been processed. It includes methods for use by both the host (prover) and
/// the guest (zkVM).
pub trait DaApp {
    type Blockhash: BlockHashTrait;

    type Address: AddressTrait;
    type BlockHeader: BlockheaderTrait<Hash = Self::Blockhash>;
    type BlobTransaction: BlobTransactionTrait<Self::Address>;
    /// A proof that a set of transactions are included in a block.
    type InclusionMultiProof;
    /// A proof that a *claimed* set of transactions is complete relative to
    /// some selection function supported by the DA layer. For example, this could be a range
    /// proof for an entire Celestia namespace.
    type CompletenessProof;
    type Error: Debug;

    const ADDRESS_LENGTH: usize;
    /// The hash of the DA layer block which is the genesis of the logical chain defined by this app.
    /// This is *not* necessarily the DA layer's genesis block.
    const RELATIVE_GENESIS: Self::Blockhash;

    fn get_relevant_txs(&self, blockhash: &Self::Blockhash) -> Vec<Self::BlobTransaction>;
    fn get_relevant_txs_with_proof(
        &self,
        blockhash: &Self::Blockhash,
    ) -> (
        Vec<Self::BlobTransaction>,
        Self::InclusionMultiProof,
        Self::CompletenessProof,
    );

    fn verify_relevant_tx_list(
        &self,
        blockheader: &Self::BlockHeader,
        txs: &Vec<Self::BlobTransaction>,
        inclusion_proof: Self::InclusionMultiProof,
        completeness_proof: Self::CompletenessProof,
    ) -> Result<(), Self::Error>;
}

pub trait BlobTransactionTrait<Addr> {
    type Data: Buf;

    fn sender(&self) -> Addr;
    fn data(&self) -> Self::Data;
}

pub trait BlockHashTrait: Serialize + Deser + PartialEq + Debug {}
