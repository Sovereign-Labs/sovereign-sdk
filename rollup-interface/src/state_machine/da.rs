use crate::traits::{AddressTrait, BlockHeaderTrait};
use borsh::{BorshDeserialize, BorshSerialize};
use bytes::Buf;
use core::fmt::Debug;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::cmp::min;
use std::io::{Read, Write};

/// A specification for the types used by a DA layer.
pub trait DaSpec {
    /// The hash of a DA layer block
    type SlotHash: BlockHashTrait;

    /// The block header type used by the DA layer
    type BlockHeader: BlockHeaderTrait<Hash = Self::SlotHash>;

    /// The transaction type used by the DA layer.
    type BlobTransaction: BlobTransactionTrait;

    /// A proof that each tx in a set of blob transactions is included in a given block.
    type InclusionMultiProof: Serialize + DeserializeOwned;

    /// A proof that a claimed set of transactions is complete.
    /// For example, this could be a range proof demonstrating that
    /// the provided BlobTransactions represent the entire contents
    /// of Celestia namespace in a given block
    type CompletenessProof: Serialize + DeserializeOwned;

    /// The parameters of the rollup which are baked into the state-transition function.
    /// For example, this could include the namespace of the rollup on Celestia.
    type ChainParams;
}

/// A `DaVerifier` implements the logic required to create a zk proof that some data
/// has been processed.
///
/// This trait implements the required functionality to *verify* claims of the form
/// "If X is the most recent block in the DA layer, then Y is the ordered set of transactions that must
/// be processed by the rollup."
pub trait DaVerifier {
    /// The set of types required by the DA layer.
    type Spec: DaSpec;

    /// The error type returned by the DA layer's verification function
    /// TODO: Should we add `std::Error` bound so it can be `()?` ?
    type Error: Debug;

    fn new(params: <Self::Spec as DaSpec>::ChainParams) -> Self;

    /// Verify a claimed set of transactions against a block header.
    fn verify_relevant_tx_list(
        &self,
        block_header: &<Self::Spec as DaSpec>::BlockHeader,
        txs: &[<Self::Spec as DaSpec>::BlobTransaction],
        inclusion_proof: <Self::Spec as DaSpec>::InclusionMultiProof,
        completeness_proof: <Self::Spec as DaSpec>::CompletenessProof,
    ) -> Result<(), Self::Error>;
}

// Simple structure that implements the Read trait for a buffer and  counts the number of bytes read from the beginning
// Useful for the partial blob reading optimization: we know for each blob how many bytes have been read from the beginning
// Because of soundness issues we cannot implement the Buf trait because the prover could get unproved blob data using the chunk method.
#[derive(Debug, Clone, Serialize, Deserialize, BorshDeserialize, BorshSerialize, PartialEq)]
pub struct BufReaderWithCounter<B: Buf> {
    inner: B,
    acc: Vec<u8>,
    counter: usize,
}

impl<B: Buf> BufReaderWithCounter<B> {
    pub fn new(inner: B) -> Self {
        let buf_size = inner.remaining();
        BufReaderWithCounter {
            inner,
            counter: 0,
            acc: Vec::with_capacity(buf_size),
        }
    }

    pub fn counter(&self) -> usize {
        self.counter
    }

    pub fn acc(&self) -> &Vec<u8> {
        &self.acc
    }
}

impl<B: Buf> Read for BufReaderWithCounter<B> {
    // Reads the inner buf into the provided buffer, and appends the data read to inner accumulator
    fn read(&mut self, mut buf: &mut [u8]) -> std::io::Result<usize> {
        let len_before_reading = self.inner.remaining();

        buf.write_all(&self.inner.copy_to_bytes(min(buf.len(), len_before_reading)))?;

        let num_read = len_before_reading - self.inner.remaining();

        self.acc.append(&mut buf.to_vec());

        self.counter += num_read;

        Ok(num_read)
    }
}

/// A transaction on a data availability layer, including the address of the sender.
pub trait BlobTransactionTrait: Serialize + DeserializeOwned {
    type Data: Buf;
    type Address: AddressTrait;

    /// Returns the address (on the DA layer) of the entity which submitted the blob transaction
    fn sender(&self) -> Self::Address;

    /// The raw data of the blob. For example, the "calldata" of an Ethereum rollup transaction
    /// This function clones the data of the blob to an external BufWithCounter
    fn data_mut(&mut self) -> &mut BufReaderWithCounter<Self::Data>;

    fn data(&self) -> &BufReaderWithCounter<Self::Data>;

    // Returns the hash of the blob. If not provided with a hint, it is computed by hashing the blob data
    fn hash(&self) -> [u8; 32];
}

/// Trait with collection of trait bounds for a block hash.
pub trait BlockHashTrait: Serialize + DeserializeOwned + PartialEq + Debug + Send + Sync {}
