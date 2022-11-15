use bytes::Bytes;

use crate::core::traits::{self, Blockheader};
use core::fmt::Debug;

pub trait DaApp {
    type Blockhash: PartialEq + Debug;

    type Address: traits::Address + traits::AsBytes;
    type Header: Blockheader<Hash = Self::Blockhash>;
    type Transaction: TxWithSender<Self::Address>;
    /// A proof that a particular transaction is included in a block
    type InclusionProof;
    /// A proof that a set of transactions are included in a block
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

    fn get_relevant_txs(blockhash: &Self::Blockhash) -> Vec<Self::Transaction>;
    fn get_relevant_txs_with_proof(
        blockhash: &Self::Blockhash,
    ) -> (
        Vec<Self::Transaction>,
        Self::InclusionMultiProof,
        Self::CompletenessProof,
    );

    fn verify_relevant_tx_list(
        &self,
        blockhash: &Self::Header,
        txs: &Vec<Self::Transaction>,
        inclusion_proof: &Self::InclusionMultiProof,
        completeness_proof: &Self::CompletenessProof,
    ) -> Result<(), Self::Error>;
}

pub trait TxWithSender<Addr> {
    fn sender(&self) -> &Addr;
    fn data(&self) -> Bytes;
}
