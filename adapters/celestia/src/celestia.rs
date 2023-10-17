use std::ops::Range;
use std::sync::{Arc, Mutex};

use anyhow::Context;
use borsh::{BorshDeserialize, BorshSerialize};
use celestia_proto::celestia::blob::v1::MsgPayForBlobs;
use celestia_proto::cosmos::tx::v1beta1::Tx;
use celestia_types::DataAvailabilityHeader;
use prost::bytes::Buf;
use prost::Message;
use serde::{Deserialize, Serialize};
use sov_rollup_interface::da::{BlockHeaderTrait as BlockHeader, CountedBufReader, Time};
use sov_rollup_interface::services::da::SlotData;
pub use tendermint::block::Header as TendermintHeader;
use tendermint::block::Height;
use tendermint::crypto::default::Sha256;
use tendermint::merkle::simple_hash_from_byte_vectors;
use tendermint::Hash;
pub use tendermint_proto::v0_34 as celestia_tm_version;
use tendermint_proto::v0_34::types::IndexWrapper;
use tendermint_proto::Protobuf;
use tracing::debug;

use crate::shares::{BlobIterator, BlobRefIterator, NamespaceGroup};
use crate::utils::{read_varint, BoxError};
use crate::verifier::address::CelestiaAddress;
use crate::verifier::{ChainValidityCondition, TmHash, PFB_NAMESPACE};

pub const GENESIS_PLACEHOLDER_HASH: &[u8; 32] = &[255; 32];

/// A partially serialized tendermint header. Only fields which are actually inspected by
/// Jupiter are included in their raw form. Other fields are pre-encoded as protobufs.
///
/// This type was first introduced as a way to circumvent a bug in tendermint-rs which prevents
/// a tendermint::block::Header from being deserialized in most formats except JSON. However
/// it also provides a significant efficiency benefit over the standard tendermint type, which
/// performs a complete protobuf serialization every time `.hash()` is called.
#[derive(PartialEq, Eq, Debug, Clone, Serialize, Deserialize)] // TODO: , BorshDeserialize, BorshSerialize)]
pub struct CompactHeader {
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

trait EncodeTm34 {
    fn encode_to_tm34_protobuf(&self) -> Result<Vec<u8>, BoxError>;
}

impl From<TendermintHeader> for CompactHeader {
    fn from(value: TendermintHeader) -> Self {
        let data_hash = match value.data_hash {
            Hash::Sha256(value) => Some(ProtobufHash(value)),
            Hash::None => None,
        };
        Self {
            version: Protobuf::<celestia_tm_version::version::Consensus>::encode_vec(
                &value.version,
            )
            .unwrap(),
            chain_id: value.chain_id.encode_vec().unwrap(),
            height: value.height.encode_vec().unwrap(),
            time: value.time.encode_vec().unwrap(),
            last_block_id: Protobuf::<celestia_tm_version::types::BlockId>::encode_vec(
                &value.last_block_id.unwrap_or_default(),
            )
            .unwrap(),
            last_commit_hash: value.last_commit_hash.encode_vec().unwrap(),
            data_hash,
            validators_hash: value.validators_hash.encode_vec().unwrap(),
            next_validators_hash: value.next_validators_hash.encode_vec().unwrap(),
            consensus_hash: value.consensus_hash.encode_vec().unwrap(),
            app_hash: value.app_hash.encode_vec().unwrap(),
            last_results_hash: value.last_results_hash.encode_vec().unwrap(),
            evidence_hash: value.evidence_hash.encode_vec().unwrap(),
            proposer_address: value.proposer_address.encode_vec().unwrap(),
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, BorshDeserialize, BorshSerialize)]
pub struct ProtobufHash(pub [u8; 32]);

pub fn protobuf_encode(hash: &Option<ProtobufHash>) -> Vec<u8> {
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

#[derive(Debug, Clone, Serialize, Deserialize)] // TODO:, BorshSerialize, BorshDeserialize)]
pub struct CelestiaHeader {
    pub dah: DataAvailabilityHeader,
    pub header: CompactHeader,
    // #[borsh_skip]
    #[serde(skip)]
    cached_prev_hash: Arc<Mutex<Option<TmHash>>>,
}

impl PartialEq for CelestiaHeader {
    fn eq(&self, other: &Self) -> bool {
        self.dah == other.dah && self.header == other.header
    }
}

impl CelestiaHeader {
    pub fn new(dah: DataAvailabilityHeader, header: CompactHeader) -> Self {
        Self {
            dah,
            header,
            cached_prev_hash: Arc::new(Mutex::new(None)),
        }
    }

    pub fn square_size(&self) -> usize {
        self.dah.row_roots.len()
    }
}

#[derive(PartialEq, Clone, Debug, Serialize, Deserialize)] // TODO: , BorshDeserialize, BorshSerialize)]
pub struct BlobWithSender {
    pub blob: CountedBufReader<BlobIterator>,
    pub sender: CelestiaAddress,
    pub hash: [u8; 32],
}

impl BlockHeader for CelestiaHeader {
    type Hash = TmHash;

    fn prev_hash(&self) -> Self::Hash {
        let mut cached_hash = self.cached_prev_hash.lock().unwrap();
        if let Some(hash) = cached_hash.as_ref() {
            return hash.clone();
        }

        // We special case the block following genesis, since genesis has a `None` hash, which
        // we don't want to deal with. In this case, we return a specail placeholder for the
        // block "hash"
        if Height::decode_vec(&self.header.height)
            .expect("header must be validly encoded")
            .value()
            == 1
        {
            let prev_hash = TmHash(tendermint::Hash::Sha256(*GENESIS_PLACEHOLDER_HASH));
            *cached_hash = Some(prev_hash.clone());
            return prev_hash;
        }

        // In all other cases, we simply return the previous block hash parsed from the header
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

        Time::from_secs(protobuf_time.unix_timestamp())
    }
}

/// We implement [`SlotData`] for [`CelestiaHeader`] in a similar fashion as for
/// [`FilteredCelestiaBlock`](crate::types::FilteredCelestiaBlock).
impl SlotData for CelestiaHeader {
    type BlockHeader = CelestiaHeader;
    type Cond = ChainValidityCondition;

    fn hash(&self) -> [u8; 32] {
        match self.header.hash() {
            tendermint::Hash::Sha256(h) => h,
            tendermint::Hash::None => unreachable!("tendermint::Hash::None should not be possible"),
        }
    }

    fn header(&self) -> &Self::BlockHeader {
        self
    }

    fn validity_condition(&self) -> ChainValidityCondition {
        ChainValidityCondition {
            prev_hash: *self.header().prev_hash().inner(),
            block_hash: <Self as SlotData>::hash(self),
        }
    }
}

#[derive(Deserialize, Serialize, PartialEq, Debug, Clone)]
pub struct CelestiaVersion {
    pub block: u32,
}

#[derive(Deserialize, Serialize, PartialEq, Debug, Clone)]
pub struct PreviousBlock {
    pub hash: Sha2Hash,
    // TODO: add parts
}

#[derive(Deserialize, Serialize, PartialEq, Debug, Clone)]
pub struct Sha2Hash(#[serde(deserialize_with = "hex::deserialize")] pub [u8; 32]);

impl AsRef<[u8]> for Sha2Hash {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

pub fn parse_pfb_namespace(
    group: NamespaceGroup,
) -> Result<Vec<(MsgPayForBlobs, TxPosition)>, BoxError> {
    if group.shares().is_empty() {
        return Ok(vec![]);
    }
    assert_eq!(group.shares()[0].namespace(), PFB_NAMESPACE);
    let mut pfbs = Vec::new();
    for blob in group.blobs() {
        let mut data = blob.data();
        while data.has_remaining() {
            pfbs.push(next_pfb(&mut data)?)
        }
    }
    Ok(pfbs)
}

#[derive(
    Debug, PartialEq, Clone, serde::Serialize, Deserialize, BorshSerialize, BorshDeserialize,
)]
pub struct TxPosition {
    /// The half-open range of shares across which this transaction is serialized.
    /// For example a transaction which was split across shares 5,6, and 7 would have range 5..8
    pub share_range: Range<usize>,
    /// The offset into the first share at which the transaction starts
    pub start_offset: usize,
}

pub(crate) fn pfb_from_iter(data: impl Buf, pfb_len: usize) -> Result<MsgPayForBlobs, BoxError> {
    let blob_tx = IndexWrapper::decode(data.take(pfb_len)).context("failed decoding blob tx")?;
    let cosmos_tx =
        Tx::decode(&blob_tx.tx[..]).context("failed decoding cosmos tx from blob tx")?;
    let messages = cosmos_tx.body.context("No body in cosmos tx")?.messages;

    anyhow::ensure!(messages.len() == 1, "Expected 1 message in cosmos tx");
    MsgPayForBlobs::decode(&messages[0].value[..]).context("failed decoding PFB frob cosmos tx")
}

fn next_pfb(mut data: &mut BlobRefIterator) -> Result<(MsgPayForBlobs, TxPosition), BoxError> {
    let (start_idx, start_offset) = data.current_position();
    let (len, len_of_len) = read_varint(&mut data).context("failed decoding varint")?;
    debug!(
        "Decoding wrapped PFB of length {}. Stripped {} bytes of prefix metadata",
        len, len_of_len
    );

    let current_share_idx = data.current_position().0;
    let pfb = pfb_from_iter(data, len as usize)?;

    Ok((
        pfb,
        TxPosition {
            share_range: start_idx..current_share_idx + 1,
            start_offset,
        },
    ))
}

#[cfg(test)]
mod tests {
    use celestia_types::ExtendedHeader;
    use sov_rollup_interface::services::da::SlotData;
    use sov_rollup_interface::zk::ValidityCondition;

    use crate::{CelestiaHeader, CompactHeader};

    const HEADER_JSON_RESPONSES: &[&str] = &[
        include_str!("../test_data/block_with_rollup_data/header.json"),
        include_str!("../test_data/block_without_rollup_data/header.json"),
    ];

    #[test]
    fn test_validity_condition() {
        let headers: Vec<_> = HEADER_JSON_RESPONSES
            .iter()
            .map(|header| {
                let eh: ExtendedHeader = serde_json::from_str(header).unwrap();
                CelestiaHeader::new(eh.dah, eh.header.into())
            })
            .collect();

        let former_validity_cond = headers[0].validity_condition();
        let latter_validity_cond = headers[1].validity_condition();

        assert!(former_validity_cond
            .combine::<sha2::Sha256>(latter_validity_cond)
            .is_ok());
        assert!(latter_validity_cond
            .combine::<sha2::Sha256>(former_validity_cond)
            .is_err());
    }

    #[test]
    fn test_compact_header_serde() {
        for header_json in HEADER_JSON_RESPONSES {
            let original_header: ExtendedHeader = serde_json::from_str(header_json).unwrap();

            let header: CompactHeader = original_header.header.into();

            let serialized_header = postcard::to_stdvec(&header).unwrap();
            let deserialized_header: CompactHeader =
                postcard::from_bytes(&serialized_header).unwrap();
            assert_eq!(deserialized_header, header)
        }
    }

    #[test]
    fn test_compact_header_hash() {
        let expected_hashes = [
            "C839E720DA55CC6E43EC7CE00744D6151D79E84C81D7F6995F3B13B7AE532456",
            "F769490DC768E7678160384070727533B7AE809477EA5D191CF7AF5C917A7973",
        ];
        for (header_json, expected_hash) in HEADER_JSON_RESPONSES.iter().zip(expected_hashes.iter())
        {
            let original_header: ExtendedHeader = serde_json::from_str(header_json).unwrap();

            let tm_header = original_header.header.clone();
            let compact_header: CompactHeader = original_header.header.into();

            assert_eq!(tm_header.hash(), compact_header.hash());
            assert_eq!(
                hex::decode(expected_hash).unwrap(),
                compact_header.hash().as_bytes()
            );

            assert_eq!(tm_header.hash(), compact_header.hash(),);
        }
    }

    #[test]
    #[cfg(feature = "risc0")]
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
}
