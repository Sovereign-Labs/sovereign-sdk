use crate::celestia::{CompactHeader, ProtobufHash};
use crate::celestia_tm_version;
use crate::config::Network;
use crate::types::{TmHash, APP_VERSION};
use celestia_types::nmt::{NamespacedHash, NamespacedHashExt};
use celestia_types::DataAvailabilityHeader;
use jsonrpsee::core::Serialize;
use serde::{Deserialize, Serializer};
use serde_with::serde_as;
use sov_rollup_interface::common::HexHash;
use sov_rollup_interface::node::da::SubmitBlobReceipt;
use tendermint_proto::Protobuf;

#[serde_as]
#[derive(Debug, Serialize)]
pub struct SubmitBlobRequest {
    #[serde(serialize_with = "serialize_namespace_hex")]
    pub namespace: celestia_types::nmt::Namespace,
    #[serde_as(as = "serde_with::hex::Hex")]
    pub data: Vec<u8>,
    pub asynchronous: bool,
    pub network: Network,
}

fn serialize_namespace_hex<S>(
    namespace: &celestia_types::nmt::Namespace,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    // TODO: Should we use `as_bytes()` ??
    let bytes = namespace
        .id_v0()
        .ok_or_else(|| serde::ser::Error::custom("Namespace is not v0"))?;
    serializer.serialize_str(&hex::encode(bytes))
}

/// Response from <https://t.tech/docs/v0/blob/POST>
#[derive(Debug, Deserialize)]
pub struct SubmitBlobAsyncResponse {
    #[serde(rename = "twinkleRequestId")]
    pub twinkle_request_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum BlobStatus {
    Pending,
    Included {
        height: u64,
        #[serde(rename = "txId")]
        transaction_id: HexHash,
    },
    // TODO: is there some info that we can use?
    Rejected,
}

/// Response from <https://t.tech/docs/v0/blob/status/GET>
#[serde_as]
#[derive(Debug, Deserialize)]
pub struct BlobStatusResponse {
    #[serde(flatten)]
    pub status: BlobStatus,
    #[serde_as(as = "serde_with::base64::Base64")]
    pub commitment: Vec<u8>,
}

impl TryFrom<BlobStatusResponse> for SubmitBlobReceipt<TmHash> {
    type Error = anyhow::Error;

    fn try_from(value: BlobStatusResponse) -> Result<Self, Self::Error> {
        let blob_hash = value.commitment.try_into().map_err(|e: Vec<u8>| {
            anyhow::anyhow!(
                "Wrong commitment size, should 32 bytes, but was {}",
                e.len(),
            )
        })?;
        let BlobStatus::Included { transaction_id, .. } = value.status else {
            anyhow::bail!("Transaction Id is not present, is status `Included`?");
        };
        Ok(SubmitBlobReceipt {
            blob_hash: HexHash::new(blob_hash),
            da_transaction_id: TmHash(tendermint::Hash::Sha256(transaction_id.0)),
        })
    }
}

#[derive(Debug, Deserialize)]
pub struct HeaderResponse {
    pub header: TwinkleBlockHeader,
    pub dah: TwinkleDah,
}

#[serde_as]
#[derive(Debug, Deserialize)]
pub struct TwinkleDah {
    #[serde(rename = "rowRoots")]
    #[serde_as(as = "Vec<Base64NamespacedHash>")]
    row_roots: Vec<NamespacedHash>,
    #[serde(rename = "columnRoots")]
    #[serde_as(as = "Vec<Base64NamespacedHash>")]
    column_roots: Vec<NamespacedHash>,
}

struct Base64NamespacedHash;

impl serde_with::SerializeAs<NamespacedHash> for Base64NamespacedHash {
    fn serialize_as<S>(source: &NamespacedHash, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde_with::base64::Standard;
        serde_with::base64::Base64::<Standard>::serialize_as(&source.to_array(), serializer)
    }
}

impl<'de> serde_with::DeserializeAs<'de, NamespacedHash> for Base64NamespacedHash {
    fn deserialize_as<D>(deserializer: D) -> Result<NamespacedHash, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde_with::base64::Standard;
        let bytes: Vec<u8> = serde_with::base64::Base64::<Standard>::deserialize_as(deserializer)?;
        NamespacedHash::try_from(bytes.as_slice()).map_err(serde::de::Error::custom)
    }
}

impl From<TwinkleDah> for DataAvailabilityHeader {
    fn from(value: TwinkleDah) -> Self {
        let TwinkleDah {
            row_roots,
            column_roots,
        } = value;
        DataAvailabilityHeader::new(row_roots, column_roots, APP_VERSION).unwrap()
    }
}

#[serde_as]
#[derive(Debug, Deserialize)]
pub struct TwinkleBlockHeader {
    version: Version,
    #[serde(rename = "chainId")]
    chain_id: tendermint::chain::Id,
    #[serde_as(as = "serde_with::DisplayFromStr")]
    height: tendermint::block::Height,
    time: tendermint::Time,
    #[serde(rename = "lastBlockId")]
    last_block_id: BlockId,
    #[serde_as(as = "serde_with::DisplayFromStr")]
    #[serde(rename = "lastCommitHash")]
    last_commit_hash: tendermint::Hash,
    #[serde_as(as = "serde_with::DisplayFromStr")]
    #[serde(rename = "dataHash")]
    data_hash: tendermint::Hash,
    #[serde_as(as = "serde_with::DisplayFromStr")]
    #[serde(rename = "validatorsHash")]
    validators_hash: tendermint::Hash,
    #[serde_as(as = "serde_with::DisplayFromStr")]
    #[serde(rename = "nextValidatorsHash")]
    next_validators_hash: tendermint::Hash,
    #[serde_as(as = "serde_with::DisplayFromStr")]
    #[serde(rename = "consensusHash")]
    consensus_hash: tendermint::Hash,
    #[serde_as(as = "serde_with::DisplayFromStr")]
    #[serde(rename = "appHash")]
    app_hash: tendermint::Hash,
    #[serde_as(as = "serde_with::DisplayFromStr")]
    #[serde(rename = "lastResultsHash")]
    last_results_hash: tendermint::Hash,
    #[serde_as(as = "serde_with::DisplayFromStr")]
    #[serde(rename = "evidenceHash")]
    evidence_hash: tendermint::Hash,
    #[serde(rename = "proposerAddress")]
    #[serde_as(as = "serde_with::DisplayFromStr")]
    proposer_address: tendermint::account::Id,
}

impl From<TwinkleBlockHeader> for CompactHeader {
    fn from(value: TwinkleBlockHeader) -> Self {
        let TwinkleBlockHeader {
            version,
            chain_id,
            height,
            time,
            last_block_id,
            last_commit_hash,
            data_hash,
            validators_hash,
            next_validators_hash,
            consensus_hash,
            app_hash,
            last_results_hash,
            evidence_hash,
            proposer_address,
        } = value;

        let data_hash = match data_hash {
            tendermint::Hash::Sha256(value) => Some(ProtobufHash(value)),
            tendermint::Hash::None => None,
        };
        CompactHeader {
            version: Protobuf::<celestia_tm_version::version::Consensus>::encode_vec(
                tendermint::block::header::Version::from(version),
            ),
            chain_id: chain_id.encode_vec(),
            height: height.encode_vec(),
            time: time.encode_vec(),
            last_block_id: Protobuf::<celestia_tm_version::types::BlockId>::encode_vec(
                tendermint::block::Id::from(last_block_id),
            ),
            last_commit_hash: last_commit_hash.encode_vec(),
            data_hash,
            validators_hash: validators_hash.encode_vec(),
            next_validators_hash: next_validators_hash.encode_vec(),
            consensus_hash: consensus_hash.encode_vec(),
            app_hash: app_hash.encode_vec(),
            last_results_hash: last_results_hash.encode_vec(),
            evidence_hash: evidence_hash.encode_vec(),
            proposer_address: proposer_address.encode_vec(),
        }
    }
}

#[serde_as]
#[derive(Debug, Deserialize)]
pub struct Version {
    #[serde_as(as = "serde_with::DisplayFromStr")]
    pub block: u64,
    #[serde_as(as = "serde_with::DisplayFromStr")]
    pub app: u64,
}

impl From<Version> for tendermint::block::header::Version {
    fn from(value: Version) -> Self {
        Self {
            block: value.block,
            app: value.app,
        }
    }
}

#[serde_as]
#[derive(Debug, Deserialize)]
pub struct BlockId {
    #[serde_as(as = "serde_with::DisplayFromStr")]
    pub hash: tendermint::Hash,
    pub parts: PartSetHeader,
}

impl From<BlockId> for tendermint::block::Id {
    fn from(value: BlockId) -> Self {
        Self {
            hash: value.hash,
            part_set_header: value.parts.into(),
        }
    }
}

#[serde_as]
#[derive(Debug, Deserialize)]
pub struct PartSetHeader {
    pub total: u32,
    #[serde_as(as = "serde_with::DisplayFromStr")]
    pub hash: tendermint::Hash,
}

impl From<PartSetHeader> for tendermint::block::parts::Header {
    fn from(value: PartSetHeader) -> Self {
        Self::new(value.total, value.hash).expect("Invalid TwinklePartSetHeader")
    }
}
