use std::borrow::Cow;
use std::fmt;

use anyhow::Context;
use celestia_types::namespace_data::NamespaceData;
use celestia_types::nmt::Namespace;
use sov_celestia_adapter::types::TmHash;
use sov_celestia_adapter::verifier::address::CelestiaAddress;
use sov_celestia_adapter::verifier::RollupParams;
use sov_celestia_adapter::CelestiaHeader;
use sov_rollup_interface::da::{
    BlobReaderTrait, CountedBufReader, DaProof, DaSpec, DaVerifier, RelevantBlobs, RelevantProofs,
};

use crate::commitment::MAX_DATA_SIZE;
use crate::namespace::{verify_namespace, FibreRef};
use crate::{verify_rows_prefix, FibreRowsProof};

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
    type InclusionMultiProof = Vec<Option<FibreRowsProof>>;
    /// All shares of the namespace with their row proofs. They hold the Fibre references that
    /// the blobs must match one to one.
    type CompletenessProof = NamespaceData;
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

/// Checks that the relevant blobs are exactly the Fibre blobs referenced from the rollup
/// namespaces, and that the data they expose matches the referenced commitments.
#[derive(Clone)]
pub struct CelestiaFibreVerifier {
    rollup_batch_namespace: Namespace,
    rollup_proof_namespace: Namespace,
}

impl DaVerifier for CelestiaFibreVerifier {
    type Spec = CelestiaFibreSpec;
    type Error = anyhow::Error;

    fn new(params: RollupParams) -> Self {
        Self {
            rollup_batch_namespace: params.rollup_batch_namespace,
            rollup_proof_namespace: params.rollup_proof_namespace,
        }
    }

    fn verify_relevant_tx_list(
        &self,
        block_header: &CelestiaHeader,
        relevant_blobs: &RelevantBlobs<FibreBlob>,
        relevant_proofs: RelevantProofs<Vec<Option<FibreRowsProof>>, NamespaceData>,
    ) -> Result<(), Self::Error> {
        verify_namespace_blobs(
            block_header,
            self.rollup_batch_namespace,
            &relevant_blobs.batch_blobs,
            &relevant_proofs.batch,
        )
        .context("Invalid blobs in the batch namespace")?;
        verify_namespace_blobs(
            block_header,
            self.rollup_proof_namespace,
            &relevant_blobs.proof_blobs,
            &relevant_proofs.proof,
        )
        .context("Invalid blobs in the proof namespace")
    }
}

fn verify_namespace_blobs(
    header: &CelestiaHeader,
    namespace: Namespace,
    blobs: &[FibreBlob],
    proof: &DaProof<Vec<Option<FibreRowsProof>>, NamespaceData>,
) -> anyhow::Result<()> {
    let refs = verify_namespace(header, namespace, &proof.completeness_proof)?;
    verify_blobs(&refs, blobs, &proof.inclusion_proof)
}

/// Checks that `blobs` are the blobs of `refs`, in the same order.
fn verify_blobs(
    refs: &[FibreRef],
    blobs: &[FibreBlob],
    row_proofs: &[Option<FibreRowsProof>],
) -> anyhow::Result<()> {
    anyhow::ensure!(
        blobs.len() == refs.len() && row_proofs.len() == refs.len(),
        "Expected {} blobs and row proofs, got {} blobs and {} row proofs",
        refs.len(),
        blobs.len(),
        row_proofs.len()
    );
    for (index, ((fibre_ref, blob), row_proof)) in
        refs.iter().zip(blobs).zip(row_proofs).enumerate()
    {
        verify_blob(fibre_ref, blob, row_proof.as_ref())
            .with_context(|| format!("Invalid Fibre blob {index}"))?;
    }
    Ok(())
}

/// Checks that `blob` is the blob of `fibre_ref`, and that its length and the data read from it
/// are authenticated by `row_proof`.
fn verify_blob(
    fibre_ref: &FibreRef,
    blob: &FibreBlob,
    row_proof: Option<&FibreRowsProof>,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        blob.sender == fibre_ref.sender,
        "Sender {} doesn't match the reference",
        blob.sender
    );
    anyhow::ensure!(
        blob.hash == TmHash::from(fibre_ref.commitment),
        "Commitment doesn't match the reference"
    );
    let row_proof = row_proof.context("Row proof is missing")?;
    let verified = verify_rows_prefix(&fibre_ref.commitment, row_proof)?;
    anyhow::ensure!(
        blob.total_len() == verified.data_size as usize,
        "Length {} doesn't match the blob header length {}",
        blob.total_len(),
        verified.data_size
    );
    anyhow::ensure!(
        verified.data.starts_with(blob.verified_data()),
        "Read data isn't covered by the row proof"
    );
    Ok(())
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

    mod verifier {
        use super::*;
        use crate::commitment::prove_rows_prefix;
        use crate::namespace::tests::{
            fixture, last_share_ref, BATCH_NAMESPACE, FIBRE_BLOCK, PROOF_NAMESPACE,
        };

        const BLOB_LEN: usize = 10_000;
        /// More than the 59 data bytes in row 0, as rows are 64 bytes for this length.
        const BLOB_READ_LEN: usize = 500;

        struct Case {
            data: Vec<u8>,
            fibre_ref: FibreRef,
            blob: FibreBlob,
            row_proof: FibreRowsProof,
        }

        fn case() -> Case {
            let data: Vec<u8> = (0..BLOB_LEN).map(|i| (i * 13 + i / 241) as u8).collect();
            let config = celestia_client::api::fibre::BlobConfig::for_version(0).unwrap();
            let commitment = celestia_client::api::fibre::EncodedFibreBlob::new(&data, config)
                .unwrap()
                .id()
                .commitment();
            let fibre_ref = FibreRef {
                sender: CelestiaAddress::try_from([0xAB; 20].as_slice()).unwrap(),
                commitment,
            };
            let mut blob = FibreBlob::new(
                fibre_ref.sender,
                commitment,
                bytes::Bytes::from(data.clone()),
            );
            blob.advance(BLOB_READ_LEN);
            let row_proof = prove_rows_prefix(&commitment, &data, BLOB_READ_LEN).unwrap();
            Case {
                data,
                fibre_ref,
                blob,
                row_proof,
            }
        }

        fn verify_case(case: &Case) -> anyhow::Result<()> {
            verify_blobs(
                &[case.fibre_ref],
                std::slice::from_ref(&case.blob),
                &[Some(case.row_proof.clone())],
            )
        }

        fn root_cause(result: anyhow::Result<()>) -> String {
            result.unwrap_err().root_cause().to_string()
        }

        /// Rebuilds the blob from its witness form after `modify` changes it.
        fn forge(blob: &FibreBlob, modify: impl FnOnce(&mut serde_json::Value)) -> FibreBlob {
            let mut wire = serde_json::to_value(blob).unwrap();
            modify(&mut wire);
            serde_json::from_value(wire).unwrap()
        }

        #[test]
        fn accepts_read_blob_matching_reference() {
            verify_case(&case()).unwrap();
        }

        #[test]
        fn accepts_witness_blob_matching_reference() {
            let mut case = case();
            case.blob = round_trip(&case.blob);

            verify_case(&case).unwrap();
        }

        #[test]
        fn rejects_blob_from_other_sender() {
            let mut case = case();
            case.fibre_ref.sender = CelestiaAddress::try_from([0xCD; 20].as_slice()).unwrap();

            assert_eq!(
                root_cause(verify_case(&case)),
                format!("Sender {} doesn't match the reference", case.blob.sender)
            );
        }

        #[test]
        fn rejects_blob_with_other_commitment() {
            let mut case = case();
            case.blob = FibreBlob::new(
                case.fibre_ref.sender,
                [0x12; 32],
                bytes::Bytes::from(case.data.clone()),
            );

            assert_eq!(
                root_cause(verify_case(&case)),
                "Commitment doesn't match the reference"
            );
        }

        #[test]
        fn rejects_missing_row_proof() {
            let case = case();

            let result = verify_blobs(&[case.fibre_ref], &[case.blob], &[None]);

            assert_eq!(root_cause(result), "Row proof is missing");
        }

        #[test]
        fn rejects_row_proof_of_other_blob() {
            let mut case = case();
            case.row_proof.rows[100] ^= 0x01;

            assert_eq!(
                root_cause(verify_case(&case)),
                crate::FibreProofError::CommitmentMismatch.to_string()
            );
        }

        #[test]
        fn rejects_forged_total_len() {
            let mut case = case();
            case.blob = forge(&case.blob, |wire| wire["total_len"] = (BLOB_LEN + 1).into());

            assert_eq!(
                root_cause(verify_case(&case)),
                format!(
                    "Length {} doesn't match the blob header length {BLOB_LEN}",
                    BLOB_LEN + 1
                )
            );
        }

        #[test]
        fn rejects_forged_read_data() {
            let mut case = case();
            case.blob = forge(&case.blob, |wire| {
                let byte = &mut wire["accumulator"][BLOB_READ_LEN - 1];
                *byte = (byte.as_u64().unwrap() ^ 0x01).into();
            });

            assert_eq!(
                root_cause(verify_case(&case)),
                "Read data isn't covered by the row proof"
            );
        }

        #[test]
        fn rejects_read_beyond_row_proof() {
            let mut case = case();
            case.row_proof = prove_rows_prefix(&case.fibre_ref.commitment, &case.data, 0).unwrap();

            assert_eq!(
                root_cause(verify_case(&case)),
                "Read data isn't covered by the row proof"
            );
        }

        #[test]
        fn rejects_missing_blob() {
            let case = case();

            let result = verify_blobs(&[case.fibre_ref], &[], &[]);

            assert_eq!(
                root_cause(result),
                "Expected 1 blobs and row proofs, got 0 blobs and 0 row proofs"
            );
        }

        #[test]
        fn rejects_extra_blob() {
            let case = case();

            let result = verify_blobs(&[], &[case.blob], &[Some(case.row_proof)]);

            assert_eq!(
                root_cause(result),
                "Expected 0 blobs and row proofs, got 1 blobs and 1 row proofs"
            );
        }

        fn verify_fixture(
            batch_blobs: Vec<FibreBlob>,
            batch_row_proofs: Vec<Option<FibreRowsProof>>,
        ) -> anyhow::Result<()> {
            let block = fixture(FIBRE_BLOCK);
            let verifier = CelestiaFibreVerifier::new(RollupParams {
                rollup_batch_namespace: BATCH_NAMESPACE,
                rollup_proof_namespace: PROOF_NAMESPACE,
            });
            verifier.verify_relevant_tx_list(
                &block.header.into(),
                &RelevantBlobs {
                    batch_blobs,
                    proof_blobs: vec![],
                },
                RelevantProofs {
                    batch: DaProof {
                        inclusion_proof: batch_row_proofs,
                        completeness_proof: block.batch_rows,
                    },
                    proof: DaProof {
                        inclusion_proof: vec![],
                        completeness_proof: block.proof_rows,
                    },
                },
            )
        }

        #[test]
        fn rejects_block_without_referenced_blob() {
            assert_eq!(
                root_cause(verify_fixture(vec![], vec![])),
                "Expected 1 blobs and row proofs, got 0 blobs and 0 row proofs"
            );
        }

        #[test]
        fn rejects_referenced_blob_without_row_proof() {
            let fibre_ref = last_share_ref(&fixture(FIBRE_BLOCK).batch_rows);
            let blob = FibreBlob::new(
                fibre_ref.sender,
                fibre_ref.commitment,
                bytes::Bytes::from_static(b"unknown data"),
            );

            assert_eq!(
                root_cause(verify_fixture(vec![blob], vec![None])),
                "Row proof is missing"
            );
        }

        #[test]
        fn accepts_namespace_without_references() {
            let block = fixture(FIBRE_BLOCK);
            let proof = DaProof {
                inclusion_proof: vec![],
                completeness_proof: block.proof_rows,
            };

            verify_namespace_blobs(&block.header.into(), PROOF_NAMESPACE, &[], &proof).unwrap();
        }
    }
}
