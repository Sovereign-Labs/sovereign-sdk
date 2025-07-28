use std::sync::{Arc, Mutex};

use borsh::{BorshDeserialize, BorshSerialize};
use celestia_types::{DataAvailabilityHeader, ExtendedHeader, ValidateBasicWithAppVersion};
use serde::{Deserialize, Serialize};
use sov_rollup_interface::da::{BlockHeaderTrait as BlockHeader, Time};
pub use tendermint::block::Header as TendermintHeader;
use tendermint::block::Height;
use tendermint::crypto::default::Sha256;
use tendermint::merkle::simple_hash_from_byte_vectors;
use tendermint::Hash;
use tendermint_proto::google::protobuf::Timestamp;
pub use tendermint_proto::v0_38 as celestia_tm_version;
use tendermint_proto::Protobuf;

use crate::types::{TmHash, ValidationError, APP_VERSION};

pub const GENESIS_PLACEHOLDER_HASH: &[u8; 32] = &[255; 32];

/// A partially serialized tendermint header. Only fields which are actually inspected by
/// Jupiter are included in their raw form. Other fields are pre-encoded as protobufs.
///
/// This type was first introduced as a way to circumvent a bug in tendermint-rs which prevents
/// a tendermint::block::Header from being deserialized in most formats except JSON. However
/// it also provides a significant efficiency benefit over the standard tendermint type, which
/// performs a complete protobuf serialization every time `.hash()` is called.
#[derive(PartialEq, Eq, Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) struct CompactHeader {
    /// Header version
    pub version: Vec<u8>,

    /// Chain ID
    pub chain_id: Vec<u8>,

    /// Current block height
    pub height: Vec<u8>,

    /// Current timestamp
    pub time: Vec<u8>,

    /// Previous block info
    pub last_block_id: Vec<u8>,

    /// Commit from validators from the last block
    pub last_commit_hash: Vec<u8>,

    /// Merkle root of transaction hashes
    pub data_hash: Option<ProtobufHash>,

    /// Validators for the current block
    pub validators_hash: Vec<u8>,

    /// Validators for the next block
    pub next_validators_hash: Vec<u8>,

    /// Consensus params for the current block
    pub consensus_hash: Vec<u8>,

    /// State after txs from the previous block
    pub app_hash: Vec<u8>,

    /// Root hash of all results from the txs from the previous block
    pub last_results_hash: Vec<u8>,

    /// Hash of evidence included in the block
    pub evidence_hash: Vec<u8>,

    /// Original proposer of the block
    pub proposer_address: Vec<u8>,
}

impl From<TendermintHeader> for CompactHeader {
    fn from(value: TendermintHeader) -> Self {
        let data_hash = value.data_hash.and_then(|h| match h {
            Hash::Sha256(value) => Some(ProtobufHash(value)),
            Hash::None => None,
        });
        Self {
            version: Protobuf::<celestia_tm_version::version::Consensus>::encode_vec(value.version),
            chain_id: value.chain_id.encode_vec(),
            height: value.height.encode_vec(),
            time: value.time.encode_vec(),
            last_block_id: Protobuf::<celestia_tm_version::types::BlockId>::encode_vec(
                value.last_block_id.unwrap_or_default(),
            ),
            last_commit_hash: value.last_commit_hash.unwrap().encode_vec(),
            data_hash,
            validators_hash: value.validators_hash.encode_vec(),
            next_validators_hash: value.next_validators_hash.encode_vec(),
            consensus_hash: value.consensus_hash.encode_vec(),
            app_hash: value.app_hash.encode_vec(),
            last_results_hash: value.last_results_hash.unwrap().encode_vec(),
            evidence_hash: value.evidence_hash.unwrap().encode_vec(),
            proposer_address: value.proposer_address.encode_vec(),
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, BorshDeserialize, BorshSerialize)]
pub(crate) struct ProtobufHash(pub [u8; 32]);

pub(crate) fn protobuf_encode(hash: &Option<ProtobufHash>) -> Vec<u8> {
    match hash {
        Some(ProtobufHash(value)) => prost::Message::encode_to_vec(&value.to_vec()),
        None => prost::Message::encode_to_vec(&vec![]),
    }
}

impl CompactHeader {
    /// Hash this header
    // TODO: this function can be made even more efficient. Rather than computing the block hash,
    // we could provide the hash as a non-deterministic input and simply verify the correctness of the
    // fields that we care about.
    pub fn hash(&self) -> Hash {
        // Note that if there is an encoding problem this will
        // panic (as the golang code would):
        // https://github.com/tendermint/tendermint/blob/134fe2896275bb926b49743c1e25493f6b24cc31/types/block.go#L393
        // https://github.com/tendermint/tendermint/blob/134fe2896275bb926b49743c1e25493f6b24cc31/types/encoding_helper.go#L9:6

        let encoded_data_hash = protobuf_encode(&self.data_hash);
        let fields_bytes = vec![
            &self.version,
            &self.chain_id,
            &self.height,
            &self.time,
            &self.last_block_id,
            &self.last_commit_hash,
            &encoded_data_hash,
            &self.validators_hash,
            &self.next_validators_hash,
            &self.consensus_hash,
            &self.app_hash,
            &self.last_results_hash,
            &self.evidence_hash,
            &self.proposer_address,
        ];

        Hash::Sha256(simple_hash_from_byte_vectors::<Sha256>(&fields_bytes))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CelestiaHeader {
    pub(crate) dah: DataAvailabilityHeader,
    pub(crate) header: CompactHeader,
    #[serde(skip)]
    cached_prev_hash: Arc<Mutex<Option<TmHash>>>,
}

impl PartialEq for CelestiaHeader {
    fn eq(&self, other: &Self) -> bool {
        self.dah == other.dah && self.header == other.header
    }
}

impl CelestiaHeader {
    pub(crate) fn new(dah: DataAvailabilityHeader, header: CompactHeader) -> Self {
        Self {
            dah,
            header,
            cached_prev_hash: Arc::new(Mutex::new(None)),
        }
    }

    pub(crate) fn square_width(&self) -> usize {
        self.dah.square_width() as usize
    }

    pub(crate) fn row_length(&self) -> usize {
        // It is divided by 2 because:
        // https://github.com/celestiaorg/celestia-app/blob/fd4b78483022faf90f345ea5cb3e45790820387a/pkg/da/data_availability_header.go#L26
        // DataAvailabilityHeader (DAHeader) contains the row and column roots of the
        // erasure coded version of the data in Block.Data. The original Block.Data is
        // split into shares and arranged in a square of width squareSize. Then, this
        // square is "extended" into an extended data square (EDS) of width 2*squareSize
        // by applying Reed-Solomon encoding. For details see Section 5.2 of
        // https://arxiv.org/abs/1809.09044 or the Celestia specification:
        // https://github.com/celestiaorg/celestia-specs/blob/master/src/specs/data_structures.md#availabledataheader
        self.square_width()
            .checked_div(2)
            .expect("square_width must be divisible by 2")
    }

    /// Calculates row number based on row length of given block.
    /// `share_idx` is a row relative (not namespace relative).
    pub(crate) fn calculate_row_number_for_share(&self, share_idx: usize) -> usize {
        share_idx
            .checked_div(self.row_length())
            .expect("the square size is invalid")
    }

    #[cfg(feature = "native")]
    pub(crate) fn get_row_roots_for_namespace(
        &self,
        namespace: celestia_types::nmt::Namespace,
    ) -> impl Iterator<Item = &celestia_types::nmt::NamespacedHash> {
        self.dah.row_roots().iter().filter(move |row_hash| {
            row_hash.contains::<celestia_types::nmt::NamespacedSha2Hasher>(*namespace)
        })
    }

    /// Validates that [`DataAvailabilityHeader`] is correct and not malformed.
    /// This means:
    ///  - Well-formed row_roots and column_roots for [`APP_VERSION`] being used.
    ///  - Hash of [`DataAvailabilityHeader`] matches the hash of the block header.
    ///    This validation allows trusting [`DataAvailabilityHeader::row_roots`],
    ///    as they are included in this hash.
    pub(crate) fn validate_dah(&self) -> Result<(), ValidationError> {
        self.dah.validate_basic(APP_VERSION)?;
        let data_hash = self
            .header
            .data_hash
            .as_ref()
            .ok_or(ValidationError::MissingDataHash)?;
        if self.dah.hash().as_ref() != data_hash.0 {
            return Err(ValidationError::InvalidDataRoot);
        }
        Ok(())
    }
}

impl From<ExtendedHeader> for CelestiaHeader {
    fn from(extended_header: ExtendedHeader) -> Self {
        CelestiaHeader::new(extended_header.dah, extended_header.header.into())
    }
}

impl BlockHeader for CelestiaHeader {
    type Hash = TmHash;

    fn prev_hash(&self) -> Self::Hash {
        let mut cached_hash = self.cached_prev_hash.lock().unwrap();
        if let Some(hash) = cached_hash.as_ref() {
            return hash.clone();
        }

        // Special case the block following genesis, since genesis has a `None` hash, which
        // we don't want to deal with. In this case, we return a special placeholder for the
        // block "hash"
        if Height::decode_vec(&self.header.height)
            .expect("header must be validly encoded")
            .value()
            == 1
        {
            let prev_hash = TmHash(Hash::Sha256(*GENESIS_PLACEHOLDER_HASH));
            *cached_hash = Some(prev_hash.clone());
            return prev_hash;
        }

        // In all other cases, simply return the previous block hash parsed from the header
        let hash =
            <tendermint::block::Id as Protobuf<celestia_tm_version::types::BlockId>>::decode(
                self.header.last_block_id.as_ref(),
            )
            .expect("must not call prev_hash on block with no predecessor")
            .hash;
        *cached_hash = Some(TmHash(hash));
        TmHash(hash)
    }

    fn hash(&self) -> Self::Hash {
        TmHash(self.header.hash())
    }

    fn height(&self) -> u64 {
        let height = tendermint::block::Height::decode(self.header.height.as_slice())
            .expect("Height must be valid");
        height.value()
    }

    fn time(&self) -> Time {
        let protobuf_time = tendermint::time::Time::decode(self.header.time.as_slice())
            .expect("Timestamp must be valid");

        let timestamp: Timestamp = protobuf_time.into();
        let nanos: i64 = timestamp.nanos.into();
        Time::from_millis(timestamp.seconds * 1000 + nanos / 1_000_000)
    }
}

/// We implement [`sov_rollup_interface::node::da::SlotData`] for [`CelestiaHeader`] in a similar fashion as for
/// [`FilteredCelestiaBlock`](crate::types::FilteredCelestiaBlock).
#[cfg(feature = "native")]
impl sov_rollup_interface::node::da::SlotData for CelestiaHeader {
    type BlockHeader = CelestiaHeader;

    fn hash(&self) -> [u8; 32] {
        match self.header.hash() {
            Hash::Sha256(h) => h,
            Hash::None => unreachable!("tendermint::Hash::None should not be possible"),
        }
    }

    fn header(&self) -> &Self::BlockHeader {
        self
    }

    fn timestamp(&self) -> sov_rollup_interface::da::Time {
        self.time()
    }
}

#[derive(Deserialize, Serialize, PartialEq, Debug, Clone)]
pub struct Sha2Hash(#[serde(deserialize_with = "hex::deserialize")] pub [u8; 32]);

impl AsRef<[u8]> for Sha2Hash {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use celestia_types::ExtendedHeader;

    use crate::test_helper::files::{
        with_rollup_batch_data, with_rollup_proof_data, without_rollup_batch_data,
    };
    use crate::test_helper::serialization::test_block_serialization;
    use crate::{CelestiaHeader, CompactHeader};

    const HEADER_JSON_RESPONSES: &[&str] = &[
        include_str!("../test_data/block_with_rollup_batch_data/prev_header.json"),
        include_str!("../test_data/block_with_rollup_batch_data/header.json"),
    ];

    #[test]
    fn test_compact_header_serde() {
        for header_json in HEADER_JSON_RESPONSES {
            let original_header: ExtendedHeader = serde_json::from_str(header_json).unwrap();

            let header: CompactHeader = original_header.header.into();

            let serialized_header = postcard::to_stdvec(&header).unwrap();
            let deserialized_header: CompactHeader =
                postcard::from_bytes(&serialized_header).unwrap();
            assert_eq!(deserialized_header, header);
        }
    }

    #[test]
    fn test_compact_header_hash() {
        let expected_hashes = [
            "f45a8d5efb8b64a7f1ddbdc5f24f29163ec24007dde9df62b966f2504b347fcc",
            "ff623b7e8a9075cc40fa32c64348632d80d3178e56fdef4e778ebb0a3f94f328",
        ];
        for (header_json, expected_hash) in HEADER_JSON_RESPONSES.iter().zip(expected_hashes.iter())
        {
            let original_header: ExtendedHeader = serde_json::from_str(header_json).unwrap();

            let tm_header = original_header.header.clone();
            let compact_header: CompactHeader = original_header.header.into();

            assert_eq!(tm_header.hash(), compact_header.hash());
            assert_eq!(
                expected_hash,
                &hex::encode(compact_header.hash().as_bytes()),
            );

            assert_eq!(tm_header.hash(), compact_header.hash(),);
        }
    }

    #[test]
    fn test_zkvm_serde_celestia_header() {
        // regression https://github.com/eigerco/celestia-tendermint-rs/pull/12
        for header_json in HEADER_JSON_RESPONSES {
            let original_header: ExtendedHeader = serde_json::from_str(header_json).unwrap();
            let cel_header =
                CelestiaHeader::new(original_header.dah, original_header.header.into());

            let serialized = risc0_zkvm::serde::to_vec(&cel_header).unwrap();
            let deserialized = risc0_zkvm::serde::from_slice(&serialized).unwrap();

            assert_eq!(cel_header, deserialized);
        }
    }

    #[tokio::test]
    async fn test_zkvm_serde_block_with_batch_serialization() {
        let block = with_rollup_batch_data::filtered_block();

        test_block_serialization(block).await;
    }

    #[tokio::test]
    async fn test_zkvm_serde_block_without_batch_serialization() {
        let block = without_rollup_batch_data::filtered_block();

        test_block_serialization(block).await;
    }

    #[tokio::test]
    async fn test_zkvm_serde_block_with_proof_serialization() {
        let block = with_rollup_proof_data::filtered_block();
        test_block_serialization(block).await;
    }
}
