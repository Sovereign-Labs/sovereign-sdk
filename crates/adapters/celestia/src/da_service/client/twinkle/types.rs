use crate::celestia::{CompactHeader, ProtobufHash};
use crate::celestia_tm_version;
use crate::config::{Network, TxPriority};
use crate::types::{TmHash, APP_VERSION};
use base64::engine::general_purpose;
use base64::Engine;
use celestia_types::nmt::{NamespaceProof, NamespacedHash};
use celestia_types::row_namespace_data::{NamespaceData, RowNamespaceData};
use celestia_types::{DataAvailabilityHeader, Share};
use jsonrpsee::core::Serialize;
use serde::{Deserialize, Serializer};
use serde_with::serde_as;
use sov_rollup_interface::common::HexHash;
use sov_rollup_interface::node::da::SubmitBlobReceipt;
use tendermint_proto::Protobuf;

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FeePriority {
    Slow,
    Normal,
    Fast,
}

impl From<TxPriority> for FeePriority {
    fn from(value: TxPriority) -> Self {
        match value {
            TxPriority::Low => FeePriority::Slow,
            TxPriority::Medium => FeePriority::Normal,
            TxPriority::High => FeePriority::Fast,
        }
    }
}

#[serde_as]
#[derive(Debug, Serialize)]
pub struct SubmitBlobRequest {
    #[serde(serialize_with = "serialize_namespace_hex")]
    pub namespace: celestia_types::nmt::Namespace,
    #[serde_as(as = "serde_with::hex::Hex")]
    pub data: Vec<u8>,
    pub asynchronous: bool,
    pub network: Network,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fee_priority: Option<FeePriority>,
    pub authored: bool,
}

fn serialize_namespace_hex<S>(
    namespace: &celestia_types::nmt::Namespace,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let bytes = namespace
        .id_v0()
        .ok_or_else(|| serde::ser::Error::custom("Namespace is not v0"))?;
    serializer.serialize_str(&hex::encode(bytes))
}

pub(crate) fn serialize_namespace_base_64(namespace: &celestia_types::nmt::Namespace) -> String {
    // Use bytes here opposed to `id_v0`, because this is what is expected.
    let bytes = namespace.as_bytes();
    general_purpose::STANDARD.encode(bytes)
}

fn serialize_namespaces_base64<S>(
    namespaces: &[celestia_types::nmt::Namespace],
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    use serde::ser::SerializeSeq;
    let mut seq = serializer.serialize_seq(Some(namespaces.len()))?;
    for namespace in namespaces {
        seq.serialize_element(&serialize_namespace_base_64(namespace))?;
    }
    seq.end()
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
    pub dah: TwinkleDataAvailabilityHeader,
}

#[serde_as]
#[derive(Debug, Deserialize)]
pub struct TwinkleDataAvailabilityHeader {
    #[serde(rename = "rowRoots")]
    #[serde_as(as = "Vec<serde_with::base64::Base64>")]
    raw_row_roots: Vec<Vec<u8>>,
    #[serde(rename = "columnRoots")]
    #[serde_as(as = "Vec<serde_with::base64::Base64>")]
    raw_column_roots: Vec<Vec<u8>>,
}

impl TryFrom<TwinkleDataAvailabilityHeader> for DataAvailabilityHeader {
    type Error = celestia_types::Error;

    fn try_from(value: TwinkleDataAvailabilityHeader) -> Result<Self, Self::Error> {
        let TwinkleDataAvailabilityHeader {
            raw_row_roots,
            raw_column_roots,
        } = value;
        let mut row_roots = Vec::with_capacity(raw_row_roots.len());
        for raw_row_root in raw_row_roots {
            row_roots.push(NamespacedHash::try_from(&raw_row_root[..])?);
        }
        let mut column_roots = Vec::with_capacity(raw_column_roots.len());
        for raw_column_root in raw_column_roots {
            column_roots.push(NamespacedHash::try_from(&raw_column_root[..])?);
        }

        DataAvailabilityHeader::new(row_roots, column_roots, APP_VERSION)
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

impl TryFrom<TwinkleBlockHeader> for CompactHeader {
    type Error = celestia_types::Error;

    fn try_from(value: TwinkleBlockHeader) -> Result<Self, Self::Error> {
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
        Ok(CompactHeader {
            version: Protobuf::<celestia_tm_version::version::Consensus>::encode_vec(
                tendermint::block::header::Version::from(version),
            ),
            chain_id: chain_id.encode_vec(),
            height: height.encode_vec(),
            time: time.encode_vec(),
            last_block_id: Protobuf::<celestia_tm_version::types::BlockId>::encode_vec(
                tendermint::block::Id::try_from(last_block_id)?,
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
        })
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

impl TryFrom<BlockId> for tendermint::block::Id {
    type Error = celestia_types::Error;

    fn try_from(value: BlockId) -> Result<Self, Self::Error> {
        Ok(Self {
            hash: value.hash,
            part_set_header: value.parts.try_into()?,
        })
    }
}

#[serde_as]
#[derive(Debug, Deserialize)]
pub struct PartSetHeader {
    pub total: u32,
    #[serde_as(as = "serde_with::DisplayFromStr")]
    pub hash: tendermint::Hash,
}

impl TryFrom<PartSetHeader> for tendermint::block::parts::Header {
    type Error = celestia_types::Error;

    fn try_from(value: PartSetHeader) -> Result<Self, Self::Error> {
        Self::new(value.total, value.hash).map_err(Into::into)
    }
}

#[derive(Debug, Deserialize)]
pub struct TwinkleNamespaceResponse {
    result: Vec<TwinkleRowNamespaceData>,
}

impl TryFrom<TwinkleNamespaceResponse> for NamespaceData {
    type Error = celestia_types::Error;

    fn try_from(value: TwinkleNamespaceResponse) -> Result<Self, Self::Error> {
        let TwinkleNamespaceResponse { result } = value;
        let mut rows = Vec::with_capacity(result.len());
        for row_data in result {
            rows.push(row_data.try_into()?);
        }
        Ok(NamespaceData { rows })
    }
}

#[serde_as]
#[derive(Debug, Deserialize)]
pub struct TwinkleRawRowProof {
    start: Option<i64>,
    end: i64,
    #[serde_as(as = "Vec<serde_with::base64::Base64>")]
    nodes: Vec<Vec<u8>>,
    #[serde_as(as = "Option<serde_with::base64::Base64>")]
    #[serde(rename = "leafHash")]
    // From docs: This field will be empty in case of Inclusion Proof.
    leaf_hash: Option<Vec<u8>>,
    #[serde(rename = "isMaxNamespaceIgnored")]
    is_max_namespace_ignored: bool,
}

impl From<TwinkleRawRowProof> for celestia_proto::proof::pb::Proof {
    fn from(value: TwinkleRawRowProof) -> Self {
        let TwinkleRawRowProof {
            start,
            end,
            nodes,
            leaf_hash,
            is_max_namespace_ignored,
        } = value;
        Self {
            start: start.unwrap_or_default(),
            end,
            nodes,
            leaf_hash: leaf_hash.unwrap_or_default(),
            is_max_namespace_ignored,
        }
    }
}

impl TryFrom<TwinkleRawRowProof> for NamespaceProof {
    type Error = celestia_types::Error;

    fn try_from(value: TwinkleRawRowProof) -> Result<Self, Self::Error> {
        let proto = celestia_proto::proof::pb::Proof::from(value);
        proto.try_into()
    }
}

#[serde_as]
#[derive(Debug, Deserialize)]
pub struct TwinkleRowNamespaceData {
    #[serde_as(as = "Vec<serde_with::base64::Base64>")]
    #[serde(rename = "shares")]
    raw_shares: Vec<Vec<u8>>,
    #[serde(rename = "proof")]
    raw_proof: TwinkleRawRowProof,
}

impl TryFrom<TwinkleRowNamespaceData> for RowNamespaceData {
    type Error = celestia_types::Error;

    fn try_from(value: TwinkleRowNamespaceData) -> Result<Self, Self::Error> {
        let TwinkleRowNamespaceData {
            raw_shares,
            raw_proof,
        } = value;
        let proof = NamespaceProof::try_from(raw_proof)?;
        let mut shares = Vec::with_capacity(raw_shares.len());
        for raw_share in raw_shares {
            let share = Share::from_raw(&raw_share)?;
            shares.push(share);
        }
        Ok(RowNamespaceData { proof, shares })
    }
}

#[serde_as]
#[derive(Debug, Serialize)]
pub struct GetAllBlobsRequest {
    #[serde(serialize_with = "serialize_namespaces_base64")]
    pub namespaces: Vec<celestia_types::nmt::Namespace>,
    pub network: Network,
    pub height: u64,
}

#[serde_as]
#[derive(Debug, Deserialize)]
pub struct BlobDataResponseItem {
    #[serde_as(as = "serde_with::base64::Base64")]
    pub data: Vec<u8>,
    // Ignore other fields as they are not used
    // #[serde_as(as = "serde_with::base64::Base64")]
    // pub raw_namespace: Vec<u8>,
    //     "namespace": "AAAAAAAAAAAAAAAAAAAAAAAAAN6t////////vu8=",
    //     "data": "AAAAdHdpbmtsZQ==",
    //     "shareVersion": 0,
    //     "commitment": "1s1WX41x9Ti2I9Uu0vBvxMQ5dAkr1CewS5+0kfm5Q1o=",
    //     "index": 972
}
