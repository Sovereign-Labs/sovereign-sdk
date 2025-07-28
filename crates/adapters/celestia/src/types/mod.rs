mod error;

use std::convert::Infallible;
use std::ops::Range;

use borsh::{BorshDeserialize, BorshSerialize};
/// Reexport the [`Namespace`] from `celestia-types`
pub use celestia_types::nmt::Namespace;
use celestia_types::row_namespace_data::NamespaceData;
use celestia_types::AppVersion;
pub use error::*;
use serde::{Deserialize, Serialize};
use sov_rollup_interface::common::HexHash;
use sov_rollup_interface::da::{BlobReaderTrait, BlockHashTrait, CountedBufReader};
use sov_rollup_interface::sov_universal_wallet::UniversalWallet;

use crate::shares::BlobIterator;
use crate::verifier::address::CelestiaAddress;
use crate::CelestiaHeader;

pub(crate) const APP_VERSION: AppVersion = AppVersion::V4;
pub(crate) const SUPPORTED_SHARE_VERSION: u8 = 1;

#[derive(Debug, PartialEq, PartialOrd, Ord, Clone, Eq, Hash, Serialize, Deserialize)]
pub struct TmHash(pub tendermint::Hash);

// Schema type for TmHash to enable UniversalWallet support
#[derive(UniversalWallet)]
#[allow(dead_code)]
#[doc(hidden)]
pub struct TmHashSchema(#[sov_wallet(display(hex))] [u8; 32]);

impl sov_rollup_interface::sov_universal_wallet::schema::OverrideSchema for TmHash {
    type Output = TmHashSchema;
}

impl BorshSerialize for TmHash {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> Result<(), std::io::Error> {
        BorshSerialize::serialize(self.inner(), writer)
    }
}

impl BorshDeserialize for TmHash {
    fn deserialize(buf: &mut &[u8]) -> Result<Self, std::io::Error> {
        let bytes = <[u8; 32] as BorshDeserialize>::deserialize(buf)?;
        Ok(Self(tendermint::Hash::Sha256(bytes)))
    }

    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
        let bytes = <[u8; 32]>::deserialize_reader(reader)?;
        Ok(Self(tendermint::Hash::Sha256(bytes)))
    }
}

impl AsRef<[u8]> for TmHash {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

impl core::fmt::Display for TmHash {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "0x{}", self.0)
    }
}

impl core::str::FromStr for TmHash {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let stripped = s.strip_prefix("0x").unwrap_or(s);
        let inner = tendermint::Hash::from_str(stripped)?;
        Ok(TmHash(inner))
    }
}

impl TmHash {
    pub fn inner(&self) -> &[u8; 32] {
        match self.0 {
            tendermint::Hash::Sha256(ref h) => h,
            // Hack: when the hash is None, we return a hash of all 255s as a placeholder.
            // TODO: add special casing for the genesis block at a higher level
            tendermint::Hash::None => unreachable!("Only the genesis block has a None hash, and we use a placeholder in that corner case")
        }
    }
}

impl BlockHashTrait for TmHash {}

impl From<TmHash> for [u8; 32] {
    fn from(val: TmHash) -> Self {
        *val.inner()
    }
}

impl TryFrom<[u8; 32]> for TmHash {
    type Error = Infallible;

    fn try_from(value: [u8; 32]) -> Result<Self, Self::Error> {
        Ok(Self(tendermint::Hash::Sha256(value)))
    }
}

#[derive(PartialEq, Clone, Debug, Serialize, Deserialize)]
pub struct BlobWithSender {
    pub(crate) blob: CountedBufReader<BlobIterator>,
    // Range in the entire namespace
    pub(crate) range_in_namespace: Range<usize>,
    pub(crate) sender: CelestiaAddress,
    pub hash: HexHash,
}

impl BlobReaderTrait for BlobWithSender {
    type Address = CelestiaAddress;
    type BlobHash = TmHash;

    fn sender(&self) -> CelestiaAddress {
        self.sender.clone()
    }

    fn hash(&self) -> Self::BlobHash {
        TmHash(tendermint::Hash::Sha256(self.hash.0))
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

/// Data that is required for extracting the relevant blobs from the namespace
#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub(crate) struct NamespaceRelevantData {
    /// Celestia namespace.
    pub(crate) namespace: Namespace,
    /// All relevant rollup shares as they appear in an extended data square, with proofs.
    pub(crate) data: NamespaceData,
}

impl NamespaceRelevantData {
    #[cfg(feature = "native")]
    pub(crate) fn new(namespace: Namespace, data: NamespaceData) -> Self {
        #[cfg(debug_assertions)]
        {
            for row in &data.rows {
                for share in &row.shares {
                    assert_eq!(
                        share.namespace(),
                        namespace,
                        "Share from different namespace {} (expected {})",
                        String::from_utf8_lossy(share.namespace().as_bytes()),
                        String::from_utf8_lossy(namespace.as_bytes())
                    );
                }
            }
        }
        Self { namespace, data }
    }

    #[cfg(feature = "native")]
    pub(crate) fn get_blobs_with_sender(&self) -> Vec<BlobWithSender> {
        let mut output = Vec::new();
        let ns_iterator = crate::shares::NamespaceDataIterator::new(&self.data);

        for share_seq in ns_iterator {
            #[cfg(debug_assertions)]
            {
                share_seq.check_consistency();
            }
            // Commitment
            let commitment = celestia_types::Commitment::from_shares(
                self.namespace,
                &share_seq.shares,
                APP_VERSION,
            )
            .expect("blob must be valid");
            let hash = HexHash::new(*commitment.hash());

            let range_in_namespace = share_seq.range_in_ns.clone();
            let Ok(blob) = crate::shares::Blob::try_from(share_seq) else {
                tracing::warn!("Failed to create blob from share sequence. Only can happen if support for share version above 1 is not added yet.");
                continue;
            };
            let Some(sender) = blob.signer() else {
                tracing::debug!("Blob without a signer, happens if blob is version 0");
                continue;
            };
            let blob_tx = BlobWithSender {
                blob: CountedBufReader::new(blob.into_iter()),
                range_in_namespace,
                sender,
                hash,
            };
            output.push(blob_tx);
        }
        output
    }
}

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct FilteredCelestiaBlock {
    pub(crate) header: CelestiaHeader,
    /// Batch related data.
    pub(crate) rollup_batch_data: NamespaceRelevantData,
    /// Proof related data.
    pub(crate) rollup_proof_data: NamespaceRelevantData,
}

#[cfg(feature = "native")]
impl sov_rollup_interface::node::da::SlotData for FilteredCelestiaBlock {
    type BlockHeader = CelestiaHeader;

    fn hash(&self) -> [u8; 32] {
        match self.header.header.hash() {
            tendermint::Hash::Sha256(h) => h,
            tendermint::Hash::None => {
                unreachable!("tendermint::Hash::None should not be possible")
            }
        }
    }

    fn header(&self) -> &Self::BlockHeader {
        &self.header
    }

    fn timestamp(&self) -> sov_rollup_interface::da::Time {
        use sov_rollup_interface::da::BlockHeaderTrait;
        self.header.time()
    }
}

impl FilteredCelestiaBlock {
    #[cfg(feature = "native")]
    pub(crate) fn new(
        rollup_batch_data: NamespaceRelevantData,
        rollup_proof_data: NamespaceRelevantData,
        header: celestia_types::ExtendedHeader,
    ) -> anyhow::Result<Self> {
        Ok(FilteredCelestiaBlock {
            header: CelestiaHeader::new(header.dah, header.header.into()),
            rollup_batch_data,
            rollup_proof_data,
        })
    }
}

/// Proof of the last share
#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct NamespaceBoundaryProof {
    // This should be the last share in the namespace
    pub last_share_proof: celestia_types::nmt::NamespaceProof,
    /// The last share of the namespace, if proof is of presence.
    pub last_share: Option<celestia_types::Share>,
}

#[cfg(feature = "native")]
impl NamespaceBoundaryProof {
    pub(crate) fn from_namespace_data(namespace_data: &NamespaceRelevantData) -> Option<Self> {
        let last_row = namespace_data.data.rows.last()?;
        if last_row.shares.is_empty() && last_row.proof.is_of_presence() {
            panic!("Incorrect namespace data: last row proof is of presence, but no shares");
        } else if last_row.shares.is_empty() && last_row.proof.is_of_absence() {
            return Some(Self {
                last_share_proof: last_row.proof.clone(),
                last_share: None,
            });
        }
        let all_before_last = &last_row.shares[..last_row.shares.len() - 1];
        let last_share = last_row
            .shares
            .last()
            .expect("Incorrect namespace data: missing shares from last row")
            .clone();
        let last_share_proof = last_row
            .proof
            .narrow_range(all_before_last, &[], *namespace_data.namespace)
            .expect("Incorrect namespace data: cannot narrow range proof last share");
        Some(Self {
            last_share_proof: last_share_proof.into(),
            last_share: Some(last_share),
        })
    }
}

#[cfg(test)]
pub mod tests {
    use std::str::FromStr;

    use crate::test_helper::files::*;
    use crate::test_helper::ROLLUP_BATCH_NAMESPACE;
    use crate::types::{NamespaceData, NamespaceRelevantData, TmHash};

    fn test_serialize_roundtrip(raw: [u8; 32]) {
        let tm_hash = TmHash::try_from(raw).unwrap();
        let serde_serialized = serde_json::to_string(&tm_hash).unwrap();
        let serde_deserialized: TmHash = serde_json::from_str(&serde_serialized).unwrap();

        assert_eq!(tm_hash, serde_deserialized);

        let borsh_serialized = borsh::to_vec(&tm_hash).unwrap();
        let borsh_deserialized: TmHash = borsh::from_slice(&borsh_serialized).unwrap();

        assert_eq!(tm_hash, borsh_deserialized);
    }

    fn test_str_roundtrip(raw: [u8; 32]) {
        let tm_hash = TmHash::try_from(raw).unwrap();
        let s = tm_hash.to_string();
        let restored = TmHash::from_str(&s).expect("TmHash::from_str failed");

        assert_eq!(tm_hash, restored);
    }

    #[test_strategy::proptest]
    fn proptest_str_roundtrip(raw: [u8; 32]) {
        test_str_roundtrip(raw);
    }

    #[test_strategy::proptest]
    fn proptest_serde_roundtrip(raw: [u8; 32]) {
        test_serialize_roundtrip(raw);
    }

    #[test]
    fn filtered_block_with_proof_data() {
        let block = with_rollup_proof_data::filtered_block();

        // valid dah
        block.header.validate_dah().unwrap();

        let rollup_proof_data = block.rollup_proof_data;

        assert_eq!(rollup_proof_data.data.rows.len(), 1);
        assert_eq!(rollup_proof_data.data.rows[0].shares.len(), 1);
        assert!(rollup_proof_data.data.rows[0].proof.is_of_presence());
    }

    #[test]
    fn filtered_block_with_batch_data() {
        let block = with_rollup_batch_data::filtered_block();
        // valid dah
        block.header.validate_dah().unwrap();

        let rollup_batch_data = block.rollup_batch_data;

        // single rollup share
        // assert_eq!(rollup_batch_data.group.shares().len(), 1);
        assert_eq!(rollup_batch_data.data.rows.len(), 1);
        assert_eq!(rollup_batch_data.data.rows[0].shares.len(), 1);
        assert!(rollup_batch_data.data.rows[0].proof.is_of_presence());
    }

    #[test]
    fn filtered_block_without_batch_data() {
        let block = without_rollup_batch_data::filtered_block();

        // valid dah
        block.header.validate_dah().unwrap();

        let rollup_batch_data = block.rollup_batch_data;

        // no rollup shares
        // we still get a single row, but with absence proof and no shares
        assert_eq!(rollup_batch_data.data.rows.len(), 1);
        assert_eq!(rollup_batch_data.data.rows[0].shares.len(), 0);
        assert!(rollup_batch_data.data.rows[0].proof.is_of_absence());
    }

    #[test]
    fn test_get_rollup_data() {
        let path = make_test_path(with_rollup_batch_data::DATA_PATH);
        let rows: NamespaceData = load_from_file(&path, ROLLUP_BATCH_ROWS_JSON).unwrap();

        let ns_data = NamespaceRelevantData::new(ROLLUP_BATCH_NAMESPACE, rows);

        let blobs = ns_data.get_blobs_with_sender();
        assert_eq!(1, blobs.len());
        let blob = &blobs[0];

        // this is a batch submitted by sequencer, consisting of a single
        // "CreateToken" transaction, but we verify only length there to
        // not make this test depend on deserialization logic
        assert_eq!(blob.blob.total_len(), 277);
    }
}
