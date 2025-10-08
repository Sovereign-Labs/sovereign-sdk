use crate::celestia::{CompactHeader, ProtobufHash};
use crate::types::TmHash;
use crate::{celestia_tm_version, TendermintHeader};
use jsonrpsee::core::Serialize;
use schemars::JsonSchema;
use serde::{Deserialize, Serializer};
use serde_with::serde_as;
use sov_rollup_interface::common::HexHash;
use sov_rollup_interface::node::da::SubmitBlobReceipt;
use tendermint_proto::Protobuf;

#[derive(Debug, Clone, Copy, PartialEq, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Network {
    Mocha,
    Mainnet,
}

impl Network {
    fn as_str(&self) -> &str {
        match self {
            Network::Mocha => "mocha-4",
            Network::Mainnet => "mainnet",
        }
    }
}

impl std::fmt::Display for Network {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl serde::Serialize for Network {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

#[derive(Debug, Serialize)]
pub struct SubmitBlobRequest {
    // Hex-encoded
    pub namespace: String,
    // Hex-encoded
    pub data: String,
    pub asynchronous: bool,
    pub network: Network,
}

#[derive(Debug, Deserialize)]
pub struct SubmitBlobAsyncResponse {
    #[serde(rename = "twinkleRequestId")]
    pub twinkle_request_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BlobStatus {
    Pending,
    Included,
    Rejected,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum BlobStatusNice {
    Pending,
    Included {
        height: u64,
        #[serde(rename = "txId")]
        transaction_id: HexHash,
    },
    Rejected,
}

#[serde_as]
#[derive(Debug, Deserialize)]
pub struct BlobStatusResponse {
    #[serde(flatten)]
    pub status: BlobStatusNice,
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
        let BlobStatusNice::Included { transaction_id, .. } = value.status else {
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

impl From<TwinkleBlockHeader> for TendermintHeader {
    fn from(_value: TwinkleBlockHeader) -> Self {
        todo!()
    }
}

impl From<Version> for tendermint::block::header::Version {
    fn from(value: Version) -> Self {
        Self {
            block: value.block,
            app: value.app,
        }
    }
}

impl From<PartSetHeader> for tendermint::block::parts::Header {
    fn from(value: PartSetHeader) -> Self {
        Self::new(value.total, value.hash).expect("Invalid TwinklePartSetHeader")
    }
}

impl From<BlockId> for tendermint::block::Id {
    fn from(value: BlockId) -> Self {
        Self {
            hash: value.hash,
            part_set_header: value.parts.into(),
        }
    }
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

#[serde_as]
#[derive(Debug, Deserialize)]
pub struct BlockId {
    #[serde_as(as = "serde_with::DisplayFromStr")]
    pub hash: tendermint::Hash,
    pub parts: PartSetHeader,
}

#[serde_as]
#[derive(Debug, Deserialize)]
pub struct PartSetHeader {
    pub total: u32,
    #[serde_as(as = "serde_with::DisplayFromStr")]
    pub hash: tendermint::Hash,
}
