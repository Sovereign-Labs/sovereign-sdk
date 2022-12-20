use bytes::{Buf, Bytes};

use crate::core::traits::{self, Block, Blockheader, Transaction};
use core::fmt::Debug;
use std::future::Future;

/// A DaApp implements the logic required to create a zk proof that some data
/// has been processed. It includes methods for use by both the host (prover) and
/// the guest (zkVM).
pub trait DaApp {
    type Blockhash: PartialEq + Debug;

    type Address: traits::Address;
    type Header: Blockheader<Hash = Self::Blockhash>;
    type BlobTransaction: TxWithSender<Self::Address>;
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
        blockheader: &Self::Header,
        txs: &Vec<Self::BlobTransaction>,
        inclusion_proof: Self::InclusionMultiProof,
        completeness_proof: Self::CompletenessProof,
    ) -> Result<(), Self::Error>;
}

// TODO: Rename to clarify distinction with DaApp
/// A DaService is the local side of an RPC connection talking to node of the DA layer
/// It is *not* part of the logic that is zk-proven. Rather, it provides functionality
/// to allow the sovereign SDK to interact with the DA layer's RPC network.
pub trait DaService {
    /// An L1 block, possibly excluding some irrelevant information
    type FilteredBlock;
    type Future<T>: Future<Output = Result<T, Self::Error>>;
    // /// A transaction on the L1
    // type Transaction;
    // type Address;
    type Error;

    /// Retrieve the data for the given height, waiting for it to be
    /// finalized if necessary. The block, once returned, must not be reverted
    /// without a consensus violation.
    fn get_finalized_at(height: usize) -> Self::Future<Self::FilteredBlock>;

    /// Get the block at the given height, waiting for one to be mined if necessary.
    /// The returned block may not be final, and can be reverted without a consensus violation
    fn get_block_at(height: usize) -> Self::Future<Self::FilteredBlock>;

    // TODO: Consider adding the send_transaction method
    // fn send_transaction(tx: Self::Transaction, sender: Self::Address)
}

pub trait TxWithSender<Addr> {
    type Data: Buf;

    fn sender(&self) -> Addr;
    fn data(&self) -> Self::Data;
}
