//! Defines traits and types used by the rollup to verify claims about the
//! DA layer.
use core::fmt::Debug;
use std::fmt::Display;

use borsh::{BorshDeserialize, BorshSerialize};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sov_universal_wallet::UniversalWallet;

use crate as sov_rollup_interface; // Needed for UniversalWallet, as it requires global paths
use crate::BasicAddress;

/// The blob hash type of a given [`DaSpec`].
pub type DaBlobHash<Da> = <<Da as DaSpec>::BlobTransaction as BlobReaderTrait>::BlobHash;

/// A specification for the types used by a DA layer.
pub trait DaSpec:
    'static + Default + Debug + PartialEq + Eq + Clone + Send + Sync + BorshSerialize + BorshDeserialize
{
    /// The hash of a DA layer block
    type SlotHash: BlockHashTrait;

    /// The block header type used by the DA layer
    type BlockHeader: BlockHeaderTrait<Hash = Self::SlotHash> + Send + Sync;

    /// The transaction type used by the DA layer.
    type BlobTransaction: BlobReaderTrait<Address = Self::Address> + Send + Sync;

    /// How transactions can be identified on the DA layer.
    type TransactionId: Serialize + DeserializeOwned + Clone + Debug + Display + Send + Sync;

    /// The type used to represent addresses on the DA layer.
    type Address: BasicAddress + Send + Sync;

    /// A proof that each tx in a set of blob transactions is included in a given block.
    type InclusionMultiProof: Serialize + DeserializeOwned + Send + Sync;

    /// A proof that a claimed set of transactions is complete.
    /// For example, this could be a range proof demonstrating that
    /// the provided BlobTransactions represent the entire contents
    /// of Celestia namespace in a given block
    type CompletenessProof: Serialize + DeserializeOwned + Send + Sync;

    /// The parameters of the rollup which are baked into the state-transition function.
    /// For example, this could include the namespace of the rollup on Celestia.
    type ChainParams: Send + Sync;
}

/// A `DaVerifier` implements the logic required to create a zk proof that some data
/// has been processed.
///
/// This trait implements the required functionality to *verify* claims of the form
/// "If X is the most recent block in the DA layer, then Y is the ordered set of transactions that must
/// be processed by the rollup."
pub trait DaVerifier: Send + Sync {
    /// The set of types required by the DA layer.
    type Spec: DaSpec;

    /// The error type returned by the DA layer's verification function
    type Error: Debug;

    /// Create a new da verifier with the given chain parameters
    fn new(params: <Self::Spec as DaSpec>::ChainParams) -> Self;

    /// Verify a claimed set of transactions against a block header.
    fn verify_relevant_tx_list(
        &self,
        block_header: &<Self::Spec as DaSpec>::BlockHeader,
        relevant_blobs: &RelevantBlobs<<Self::Spec as DaSpec>::BlobTransaction>,
        relevant_proofs: RelevantProofs<
            <Self::Spec as DaSpec>::InclusionMultiProof,
            <Self::Spec as DaSpec>::CompletenessProof,
        >,
    ) -> Result<(), Self::Error>;
}

#[derive(Debug, Clone, Serialize, Deserialize, BorshDeserialize, BorshSerialize, PartialEq)]
/// Simple structure that implements the Read trait for a buffer and  counts the number of bytes read from the beginning.
/// Useful for the partial blob reading optimization: we know for each blob how many bytes have been read from the beginning.
///
/// Because of soundness issues, we cannot implement the Buf trait because the prover could get unproved blob data using the chunk method.
pub struct CountedBufReader<B: bytes::Buf> {
    /// The original blob data.
    inner: B,

    /// An accumulator that stores the data read from the blob buffer into a vector.
    /// Allows easy access to the data that has already been read
    accumulator: Vec<u8>,
}

impl<B: bytes::Buf> CountedBufReader<B> {
    /// Creates a new buffer reader with counter from an objet that implements the buffer trait
    pub fn new(inner: B) -> Self {
        let buf_size = inner.remaining();
        CountedBufReader {
            inner,
            accumulator: Vec::with_capacity(buf_size),
        }
    }

    /// Advance the accumulator by `num_bytes` bytes. If `num_bytes` is greater than the length
    /// of remaining unverified data, then all remaining unverified data is added to the accumulator.
    pub fn advance(&mut self, num_bytes: usize) {
        let requested = num_bytes;
        let remaining = self.inner.remaining();
        if remaining == 0 {
            return;
        }
        // `Buf::advance` would panic if `num_bytes` was greater than the length of the remaining unverified data,
        // but we just advance to the end of the buffer.
        let num_to_read = core::cmp::min(remaining, requested);
        // Extend the inner vector with zeros (copy_to_slice requires the vector to have
        // the correct *length* not just capacity)
        self.accumulator
            .resize(self.accumulator.len().saturating_add(num_to_read), 0);

        // Use copy_to_slice to overwrite the zeros we just added
        let accumulator_len = self.accumulator.len();
        let accumulator_idx = accumulator_len
            .checked_sub(num_to_read)
            .expect("invalid length");
        self.inner
            .copy_to_slice(self.accumulator[accumulator_idx..].as_mut());
    }

    /// Getter: returns a reference to an accumulator of the blob data read by the rollup
    pub fn accumulator(&self) -> &[u8] {
        &self.accumulator
    }

    /// Contains the total length of the data (length already read + length remaining)
    pub fn total_len(&self) -> usize {
        self.inner
            .remaining()
            .saturating_add(self.accumulator.len())
    }
}

/// This trait wraps "blob transaction" from a data availability layer allowing partial consumption of the
/// blob data by the rollup.
///
/// The motivation for this trait is limit the amount of validation work that a rollup has to perform when
/// verifying a state transition. In general, it will often be the case that a rollup only cares about some
/// portion of the data from a blob. For example, if a blob contains a malformed transaction then the rollup
/// will slash the sequencer and exit early - so it only cares about the content of the blob up to that point.
///
/// This trait allows the DaVerifier to track which data was read by the rollup, and only verify the relevant data.
pub trait BlobReaderTrait: Serialize + DeserializeOwned + Send + Sync + 'static {
    /// The type used to represent addresses on the DA layer.
    type Address: BasicAddress;

    /// The hash type of all DA blobs (e.g. transactions and proofs).
    type BlobHash: BlockHashTrait;

    /// Returns the address (on the DA layer) of the entity which submitted the blob transaction
    fn sender(&self) -> Self::Address;

    /// Returns the hash of the blob as it appears on the DA layer
    fn hash(&self) -> Self::BlobHash;

    /// Returns a slice containing all the data accessible to the rollup at this point in time.
    /// When running in native mode, the rollup can extend this slice by calling `advance`. In zk-mode,
    /// the rollup is limited to only the verified data.
    ///
    /// Rollups should use this method in conjunction with `advance` to read only the minimum amount
    /// of data required for execution
    fn verified_data(&self) -> &[u8];

    /// Returns the total number of bytes in the blob. Note that this may be unequal to `verified_data.len()`.
    fn total_len(&self) -> usize;

    /// Extends the `partial_data` accumulator with the next `num_bytes` of  data from the blob
    /// and returns a reference to the entire contents of the blob up to this point.
    ///
    /// If `num_bytes` is greater than the length of the remaining unverified data,
    /// then all remaining unverified data is added to the accumulator.
    ///
    /// ### Note:
    /// This method is only available when the `native` feature is enabled because it is unsafe to access
    /// unverified data during execution
    #[cfg(feature = "native")]
    fn advance(&mut self, num_bytes: usize) -> &[u8];

    /// Verifies all remaining unverified data in the blob and returns a reference to the entire contents of the blob.
    /// For efficiency, rollups should prefer use of `verified_data` and `advance` unless they know that all of the
    /// blob data will be required for execution.
    #[cfg(feature = "native")]
    fn full_data(&mut self) -> &[u8] {
        self.advance(self.total_len() - self.verified_data().len())
    }
}

/// Trait with a collection of trait bounds for a block hash.
pub trait BlockHashTrait:
    Serialize
    + DeserializeOwned
    + PartialEq
    + Debug
    + Send
    + Sync
    + Clone
    + Eq
    + Into<[u8; 32]>
    + TryFrom<[u8; 32], Error: std::error::Error + Send + Sync>
    + AsRef<[u8]>
    + core::hash::Hash
    + core::fmt::Display
    // Warning: `FromStr` and `ToString` are will be removed in future. 
    // See https://github.com/Sovereign-Labs/sovereign-sdk-wip/issues/1286 for more details.
    + core::str::FromStr
    + ToString
    + BorshSerialize
    + BorshDeserialize
{
}

/// A block header, typically used in the context of an underlying DA blockchain.
pub trait BlockHeaderTrait: PartialEq + Debug + Clone + Serialize + DeserializeOwned {
    /// Each block header must have a unique canonical hash.
    type Hash: BlockHashTrait;

    /// Each block header must contain the hash of the previous block.
    fn prev_hash(&self) -> Self::Hash;

    /// Hash the type to get the digest.
    fn hash(&self) -> Self::Hash;

    /// The current header height
    fn height(&self) -> u64;

    /// The timestamp of the block
    fn time(&self) -> Time;

    /// Returns displayable version of the header
    fn display(&self) -> impl core::fmt::Display
    where
        Self: Sized,
    {
        BlockHeaderDisplay { header: self }
    }
}

/// Wrapper of [`BlockHeaderTrait`] that implements the [`std::fmt::Display`] trait.
struct BlockHeaderDisplay<'a, T> {
    header: &'a T,
}

impl<T: BlockHeaderTrait> core::fmt::Display for BlockHeaderDisplay<'_, T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "{} prev_hash={} hash={} height={}",
            core::any::type_name::<T>(),
            self.header.prev_hash(),
            self.header.hash(),
            self.header.height()
        )
    }
}

#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    BorshDeserialize,
    BorshSerialize,
    Default,
    Ord,
    PartialOrd,
    Serialize,
    Deserialize,
    UniversalWallet,
)]
#[cfg_attr(
    feature = "arbitrary",
    derive(proptest_derive::Arbitrary, arbitrary::Arbitrary)
)]
#[serde(transparent)]
/// A timestamp, represented as seconds since the unix epoch.
pub struct Time {
    /// The number of seconds since the unix epoch
    millis: i64,
}

impl Time {
    /// Get the current time
    pub fn now() -> Self {
        let current_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("Time went backwards");
        Time {
            millis: current_time.as_millis() as i64,
        }
    }

    /// Create a time from the specified number of whole seconds.
    ///
    /// # Panics
    ///
    /// This function will panic if the number of seconds is greater than i64::MAX / 1000
    pub fn from_secs(secs: i64) -> Self {
        Time {
            millis: secs.checked_mul(1000).unwrap(),
        }
    }

    /// Create a time from the specified number of milliseconds.
    ///
    /// # Panics
    ///
    /// This function will panic if the number of milliseconds is greater than i64::MAX
    pub const fn from_millis(millis: i64) -> Self {
        Time { millis }
    }

    /// Returns the number of whole seconds since the epoch
    ///
    /// The returned value does not include the fractional (millisecond) part of the duration,
    /// which can be obtained using `subsec_millis`.
    pub const fn secs(&self) -> i64 {
        self.millis / 1000
    }

    /// Returns the fractional part of this [`Time`], in milliseconds.
    ///
    /// This method does not return the length of the time when represented by milliseconds.
    /// The returned number always represents a fractional portion of a second (i.e., it is less than one billion).
    pub const fn subsec_millis(&self) -> u16 {
        (self.millis % 1000) as u16
    }

    /// Returns the time as milliseconds since the unix epoch
    pub const fn as_millis(&self) -> i64 {
        self.millis
    }
}

/// Contains all the blobs from the relevant namespaces in the DA block.
#[derive(Serialize, Deserialize, BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct RelevantBlobs<B> {
    /// Blobs from the `proof` namespace.
    pub proof_blobs: Vec<B>,
    /// Blobs from the `batch` namespace.
    pub batch_blobs: Vec<B>,
}

impl<B> RelevantBlobs<B> {
    /// Iterates over the blobs in this block.
    pub fn as_iters(&mut self) -> RelevantBlobIters<&mut [B]> {
        RelevantBlobIters {
            proof_blobs: self.proof_blobs.as_mut_slice(),
            batch_blobs: self.batch_blobs.as_mut_slice(),
        }
    }
}

/// Holds iterators over the blobs in a given block.
pub struct RelevantBlobIters<I: IntoIterator> {
    /// ProofNamespace blobs.
    pub proof_blobs: I,
    /// BatchNamespace blobs.
    pub batch_blobs: I,
}

/// Contains the proofs that data from the relevant namespaces belong to a given DA block.
#[derive(Serialize, Deserialize, Debug, Clone, BorshDeserialize, BorshSerialize)]
pub struct RelevantProofs<InclusionMultiProof, CompletenessProof> {
    /// Proof for the `batch` namespace data.
    pub batch: DaProof<InclusionMultiProof, CompletenessProof>,
    /// Proof for the `proof` namespace data.
    pub proof: DaProof<InclusionMultiProof, CompletenessProof>,
}

/// A proof that a set of blobs belongs to a given DA block.
#[derive(Serialize, Deserialize, Debug, Clone, BorshDeserialize, BorshSerialize)]
pub struct DaProof<InclusionMultiProof, CompletenessProof> {
    /// A proof that each tx in a set of blob transactions is included in a given block.
    pub inclusion_proof: InclusionMultiProof,
    /// A proof that a claimed set of transactions is complete.
    pub completeness_proof: CompletenessProof,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_time_comparision() {
        let expected_before = Time::from_millis(1005);
        let expected_after = Time::from_millis(1006);

        assert!(expected_before < expected_after);
    }

    #[test]
    fn test_time_comparision_equal_seconds() {
        let expected_first = Time::from_millis(999);
        let expected_before = Time::from_secs(1);
        let expected_after = Time::from_millis(1001);

        assert!(expected_first < expected_before);
        assert!(expected_before < expected_after);
    }

    #[test]
    fn test_time_serialization_json() {
        let time = Time::from_millis(1005);
        let serialized = serde_json::to_string(&time).unwrap();
        assert_eq!(serialized, "1005");

        let deserialized: Time = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized, time);
    }

    #[test]
    fn test_time_serialization_binary() {
        let time = Time::from_millis(1005);
        let serialized = bincode::serialize(&time).unwrap();

        let deserialized: Time = bincode::deserialize(&serialized).unwrap();
        assert_eq!(deserialized, time);
    }
}
