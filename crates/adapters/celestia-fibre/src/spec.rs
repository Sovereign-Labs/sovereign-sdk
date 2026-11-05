use std::borrow::Cow;
use std::fmt;

use sov_celestia_adapter::types::TmHash;
use sov_celestia_adapter::verifier::address::CelestiaAddress;
use sov_celestia_adapter::verifier::RollupParams;
use sov_celestia_adapter::CelestiaHeader;
use sov_rollup_interface::da::{
    BlobReaderTrait, CountedBufReader, DaSpec, DaVerifier, RelevantBlobs, RelevantProofs,
};

use crate::commitment::MAX_DATA_SIZE;
use crate::FibreRowsProof;

/// [`DaSpec`] for Celestia Fibre: Celestia headers and addresses, with blobs stored off-chain.
#[derive(
    Default,
    serde::Serialize,
    serde::Deserialize,
    Debug,
    PartialEq,
    Eq,
    Clone,
    borsh::BorshDeserialize,
    borsh::BorshSerialize,
    schemars::JsonSchema,
)]
pub struct CelestiaFibreSpec;

impl DaSpec for CelestiaFibreSpec {
    type SlotHash = TmHash;
    type BlockHeader = CelestiaHeader;
    type BlobTransaction = FibreBlob;
    type TransactionId = TmHash;
    type Address = CelestiaAddress;
    /// Rows covering the read prefix of each blob, in blob order. `None` when the node couldn't
    /// rebuild the rows, which makes the block unprovable.
    // TODO: add namespace inclusion proofs of the v2 shares.
    type InclusionMultiProof = Vec<Option<FibreRowsProof>>;
    // TODO: namespace completeness proof.
    type CompletenessProof = ();
    type ChainParams = RollupParams;
}

/// Rollup blob downloaded from Fibre, identified by the commitment from its v2 share.
///
/// Serializes through [`FibreBlobWire`]: only the prefix read by the rollup and the total length
/// go into the witness, the unread rest is dropped.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(try_from = "FibreBlobWire<'static>")]
pub struct FibreBlob {
    sender: CelestiaAddress,
    hash: TmHash,
    blob: CountedBufReader<FibreData>,
}

impl FibreBlob {
    /// Wraps downloaded blob `data` posted by `sender` under the given Fibre `commitment`.
    pub fn new(sender: CelestiaAddress, commitment: [u8; 32], data: bytes::Bytes) -> Self {
        Self {
            sender,
            hash: TmHash::from(commitment),
            blob: CountedBufReader::new(FibreData::Downloaded(data)),
        }
    }
}

/// Source of the bytes of a [`FibreBlob`] that the rollup hasn't read yet.
#[derive(Debug, Clone, PartialEq)]
enum FibreData {
    /// Full blob data downloaded from Fibre.
    Downloaded(bytes::Bytes),
    /// Blob rebuilt from a pruned witness: only the number of unread bytes is known.
    /// Pruned blobs are read through the accumulator only and can't be advanced.
    Pruned { remaining: usize },
}

impl bytes::Buf for FibreData {
    fn remaining(&self) -> usize {
        match self {
            Self::Downloaded(data) => data.remaining(),
            Self::Pruned { remaining } => *remaining,
        }
    }

    fn chunk(&self) -> &[u8] {
        match self {
            Self::Downloaded(data) => data.chunk(),
            Self::Pruned { remaining: 0 } => &[],
            Self::Pruned { .. } => {
                panic!("Bug: unread bytes of a pruned Fibre blob were requested")
            }
        }
    }

    fn advance(&mut self, count: usize) {
        match self {
            Self::Downloaded(data) => data.advance(count),
            Self::Pruned { .. } if count == 0 => {}
            Self::Pruned { .. } => panic!("Bug: a pruned Fibre blob was advanced"),
        }
    }
}

/// Pruned witness form of [`FibreBlob`]. Borrows from the blob when serializing and owns its data
/// when deserializing.
#[derive(serde::Serialize, serde::Deserialize)]
struct FibreBlobWire<'a> {
    sender: Cow<'a, CelestiaAddress>,
    hash: Cow<'a, TmHash>,
    /// Prefix of the blob data read by the rollup.
    accumulator: Cow<'a, [u8]>,
    /// Length of the whole blob data. `u64`, so 64-bit hosts and the 32-bit guest agree on it.
    total_len: u64,
}

impl serde::Serialize for FibreBlob {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        FibreBlobWire {
            sender: Cow::Borrowed(&self.sender),
            hash: Cow::Borrowed(&self.hash),
            accumulator: Cow::Borrowed(self.blob.accumulator()),
            total_len: self.blob.total_len() as u64,
        }
        .serialize(serializer)
    }
}

/// Reasons a [`FibreBlobWire`] doesn't describe a valid blob.
#[derive(Debug, PartialEq)]
enum PrunedFibreBlobError {
    /// Fibre blobs can't be this large. Also keeps the length within the guest's `usize`.
    TotalLenTooLarge(u64),
    /// More bytes were read than the blob has.
    AccumulatorExceedsTotalLen {
        accumulator_len: usize,
        total_len: u64,
    },
}

impl fmt::Display for PrunedFibreBlobError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TotalLenTooLarge(total_len) => write!(
                f,
                "pruned Fibre blob length {total_len} exceeds the maximum {MAX_DATA_SIZE}"
            ),
            Self::AccumulatorExceedsTotalLen {
                accumulator_len,
                total_len,
            } => write!(
                f,
                "pruned Fibre blob has {accumulator_len} read bytes but length {total_len}"
            ),
        }
    }
}

impl<'a> TryFrom<FibreBlobWire<'a>> for FibreBlob {
    type Error = PrunedFibreBlobError;

    fn try_from(wire: FibreBlobWire<'a>) -> Result<Self, Self::Error> {
        let total_len = usize::try_from(wire.total_len)
            .ok()
            .filter(|total_len| *total_len <= MAX_DATA_SIZE)
            .ok_or(PrunedFibreBlobError::TotalLenTooLarge(wire.total_len))?;
        let accumulator = wire.accumulator.into_owned();
        let remaining = total_len.checked_sub(accumulator.len()).ok_or(
            PrunedFibreBlobError::AccumulatorExceedsTotalLen {
                accumulator_len: accumulator.len(),
                total_len: wire.total_len,
            },
        )?;
        Ok(Self {
            sender: wire.sender.into_owned(),
            hash: wire.hash.into_owned(),
            blob: CountedBufReader::from_raw_parts(FibreData::Pruned { remaining }, accumulator),
        })
    }
}

impl BlobReaderTrait for FibreBlob {
    type Address = CelestiaAddress;
    type BlobHash = TmHash;

    fn sender(&self) -> Self::Address {
        self.sender
    }

    fn hash(&self) -> Self::BlobHash {
        self.hash.clone()
    }

    fn verified_data(&self) -> &[u8] {
        self.blob.accumulator()
    }

    fn total_len(&self) -> usize {
        self.blob.total_len()
    }

    #[cfg(feature = "native")]
    fn advance(&mut self, num_bytes: usize) -> &[u8] {
        self.blob.advance(num_bytes);
        self.verified_data()
    }
}

/// Placeholder verifier that rejects every block until Fibre blob proofs are implemented.
#[derive(Clone)]
pub struct CelestiaFibreVerifier;

impl DaVerifier for CelestiaFibreVerifier {
    type Spec = CelestiaFibreSpec;
    type Error = anyhow::Error;

    fn new(_params: RollupParams) -> Self {
        Self
    }

    fn verify_relevant_tx_list(
        &self,
        _block_header: &CelestiaHeader,
        _relevant_blobs: &RelevantBlobs<FibreBlob>,
        _relevant_proofs: RelevantProofs<Vec<Option<FibreRowsProof>>, ()>,
    ) -> Result<(), Self::Error> {
        // Fail closed: row proofs alone don't show that the blobs are exactly the v2 shares of the
        // rollup namespace, that needs namespace inclusion and completeness proofs.
        anyhow::bail!("Celestia Fibre blob verification is not implemented")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DATA_LEN: usize = 100_000;
    const READ_LEN: usize = 1_234;

    fn read_blob() -> (FibreBlob, Vec<u8>) {
        let data: Vec<u8> = (0..DATA_LEN).map(|i| (i * 7 + i / 251) as u8).collect();
        let sender = CelestiaAddress::try_from([0xAB; 20].as_slice()).unwrap();
        let mut blob = FibreBlob::new(sender, [0x12; 32], bytes::Bytes::from(data.clone()));
        blob.advance(READ_LEN);
        (blob, data)
    }

    fn round_trip(blob: &FibreBlob) -> FibreBlob {
        serde_json::from_value(serde_json::to_value(blob).unwrap()).unwrap()
    }

    #[test]
    fn pruned_blob_keeps_read_prefix() {
        let (blob, data) = read_blob();

        assert_eq!(round_trip(&blob).verified_data(), &data[..READ_LEN]);
    }

    #[test]
    fn pruned_blob_keeps_total_len() {
        let (blob, _) = read_blob();

        assert_eq!(round_trip(&blob).total_len(), DATA_LEN);
    }

    #[test]
    fn pruned_blob_keeps_sender_and_hash() {
        let (blob, _) = read_blob();
        let pruned = round_trip(&blob);

        assert_eq!(
            (pruned.sender(), pruned.hash()),
            (blob.sender(), blob.hash())
        );
    }

    #[test]
    fn serialized_blob_omits_unread_data() {
        let (blob, _) = read_blob();

        let serialized = serde_json::to_value(&blob).unwrap();

        assert_eq!(
            serialized["accumulator"].as_array().unwrap().len(),
            READ_LEN,
            "Only the read prefix should be serialized"
        );
    }

    fn deserialize_with_total_len(total_len: u64) -> Result<FibreBlob, String> {
        let (blob, _) = read_blob();
        let mut serialized = serde_json::to_value(&blob).unwrap();
        serialized["total_len"] = total_len.into();
        serde_json::from_value(serialized).map_err(|error| error.to_string())
    }

    #[test]
    fn rejects_read_prefix_longer_than_blob() {
        let error = deserialize_with_total_len(READ_LEN as u64 - 1).unwrap_err();

        assert_eq!(
            error,
            PrunedFibreBlobError::AccumulatorExceedsTotalLen {
                accumulator_len: READ_LEN,
                total_len: READ_LEN as u64 - 1,
            }
            .to_string()
        );
    }

    #[test]
    fn rejects_blob_longer_than_fibre_maximum() {
        let total_len = MAX_DATA_SIZE as u64 + 1;

        let error = deserialize_with_total_len(total_len).unwrap_err();

        assert_eq!(
            error,
            PrunedFibreBlobError::TotalLenTooLarge(total_len).to_string()
        );
    }
}
