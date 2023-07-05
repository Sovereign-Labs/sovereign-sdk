use std::collections::HashMap;

use anyhow::ensure;
use borsh::{BorshDeserialize, BorshSerialize};
pub use nmt_rs::NamespaceId;
use serde::{Deserialize, Serialize};
use sov_rollup_interface::services::da::SlotData;
use sov_rollup_interface::Bytes;
use tendermint::crypto::default::Sha256;
use tendermint::merkle;

use crate::pfb::MsgPayForBlobs;
use crate::shares::{NamespaceGroup, Share};
use crate::utils::BoxError;
use crate::verifier::PARITY_SHARES_NAMESPACE;
use crate::{CelestiaHeader, TxPosition};

#[derive(Debug, PartialEq, Clone, Deserialize, Serialize)]
pub struct RpcNamespacedShares {
    #[serde(rename = "Proof")]
    pub proof: JsonNamespaceProof,
    #[serde(rename = "Shares")]
    pub shares: Vec<Share>,
}

#[derive(Debug, PartialEq, Clone, Deserialize, Serialize)]
pub struct JsonNamespaceProof {
    #[serde(rename = "Start")]
    start: usize,
    #[serde(rename = "End")]
    end: usize,
    #[serde(rename = "Nodes")]
    nodes: Option<Vec<StringWrapper>>,
}

#[derive(Debug, PartialEq, Clone, Deserialize, Serialize)]
pub struct ExtendedDataSquare {
    pub data_square: Vec<Share>,
    pub codec: String,
}

impl ExtendedDataSquare {
    pub fn square_size(&self) -> Result<usize, BoxError> {
        let len = self.data_square.len();
        let square_size = (len as f64).sqrt() as usize;
        ensure!(
            square_size * square_size == len,
            "eds size {} is not a perfect square",
            len
        );
        Ok(square_size)
    }

    pub fn rows(&self) -> Result<Vec<&[Share]>, BoxError> {
        let square_size = self.square_size()?;

        let mut output = Vec::with_capacity(square_size);
        for i in 0..square_size {
            let row_start = i * square_size;
            let row_end = (i + 1) * square_size;
            output.push(&self.data_square[row_start..row_end])
        }
        Ok(output)
    }
}

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct FilteredCelestiaBlock {
    pub header: CelestiaHeader,
    pub rollup_data: NamespaceGroup,
    /// A mapping from blob commitment to the PFB containing that commitment
    /// for each blob addressed to the rollup namespace
    pub relevant_pfbs: HashMap<Bytes, (MsgPayForBlobs, TxPosition)>,
    /// All rows in the extended data square which contain rollup data
    pub rollup_rows: Vec<Row>,
    /// All rows in the extended data square which contain pfb data
    pub pfb_rows: Vec<Row>,
}

impl SlotData for FilteredCelestiaBlock {
    type BlockHeader = CelestiaHeader;

    fn hash(&self) -> [u8; 32] {
        match self.header.header.hash() {
            tendermint::Hash::Sha256(h) => h,
            tendermint::Hash::None => unreachable!("tendermint::Hash::None should not be possible"),
        }
    }

    fn header(&self) -> &Self::BlockHeader {
        &self.header
    }
}

impl FilteredCelestiaBlock {
    pub fn square_size(&self) -> usize {
        self.header.square_size()
    }

    pub fn get_row_number(&self, share_idx: usize) -> usize {
        share_idx / self.square_size()
    }
    pub fn get_col_number(&self, share_idx: usize) -> usize {
        share_idx % self.square_size()
    }

    pub fn row_root_for_share(&self, share_idx: usize) -> &NamespacedHash {
        &self.header.dah.row_roots[self.get_row_number(share_idx)]
    }

    pub fn col_root_for_share(&self, share_idx: usize) -> &NamespacedHash {
        &self.header.dah.column_roots[self.get_col_number(share_idx)]
    }
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum ValidationError {
    MissingDataHash,
    InvalidDataRoot,
    InvalidEtxProof(&'static str),
    MissingTx,
    InvalidRowProof,
    InvalidSigner,
}

impl CelestiaHeader {
    pub fn validate_dah(&self) -> Result<(), ValidationError> {
        let rows_iter = self.dah.row_roots.iter();
        let cols_iter = self.dah.column_roots.iter();
        let byte_vecs: Vec<&NamespacedHash> = rows_iter.chain(cols_iter).collect();
        let root = merkle::simple_hash_from_byte_vectors::<Sha256>(&byte_vecs);
        let data_hash = self
            .header
            .data_hash
            .as_ref()
            .ok_or(ValidationError::MissingDataHash)?;
        if root != data_hash.0 {
            return Err(ValidationError::InvalidDataRoot);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, BorshDeserialize, BorshSerialize)]
pub struct Row {
    pub shares: Vec<Share>,
    pub root: NamespacedHash,
}

impl Row {
    pub fn merklized(&self) -> CelestiaNmt {
        let mut nmt = CelestiaNmt::new();
        for (idx, share) in self.shares.iter().enumerate() {
            // Shares in the two left-hand quadrants are prefixed with their namespace, while parity
            // shares (in the right-hand) quadrants always have the PARITY_SHARES_NAMESPACE
            let namespace = if idx < self.shares.len() / 2 {
                share.namespace()
            } else {
                PARITY_SHARES_NAMESPACE
            };
            nmt.push_leaf(share.as_serialized(), namespace)
                .expect("shares are pushed in order");
        }
        assert_eq!(&nmt.root(), &self.root);
        nmt
    }
}

#[derive(Deserialize, Serialize, PartialEq, Debug, Clone)]
pub struct StringWrapper {
    #[serde(rename = "/")]
    pub inner: String,
}

#[derive(Deserialize, Serialize, PartialEq, Debug, Clone)]
pub struct RpcNamespacedSharesResponse(pub Option<Vec<RpcNamespacedShares>>);

use nmt_rs::simple_merkle::proof::Proof;
use nmt_rs::{
    CelestiaNmt, NamespaceProof, NamespacedHash, NamespacedSha2Hasher, NAMESPACED_HASH_LEN,
};

impl From<JsonNamespaceProof> for NamespaceProof<NamespacedSha2Hasher> {
    fn from(val: JsonNamespaceProof) -> Self {
        NamespaceProof::PresenceProof {
            proof: Proof {
                siblings: val
                    .nodes
                    .unwrap_or_default()
                    .into_iter()
                    .map(|v| ns_hash_from_b64(&v.inner))
                    .collect(),
                start_idx: val.start as u32,
            },
            ignore_max_ns: true,
        }
    }
}

fn ns_hash_from_b64(input: &str) -> NamespacedHash {
    let mut output = [0u8; NAMESPACED_HASH_LEN];
    base64::decode_config_slice(input, base64::STANDARD, &mut output[..])
        .expect("must be valid b64");
    NamespacedHash(output)
}

#[cfg(test)]
mod tests {

    // use nmt_rs::{NamespaceProof, NamespacedSha2Hasher};

    // use super::{ns_hash_from_b64, RpcNamespacedSharesResponse};

    // const ROW_ROOTS: &[&'static str] = &[
    //     "AAAAAAAAAAEAAAAAAAAAAT4A1HvHQCYkf1sQ7zmTJH11jd1Hxn+YCcC9mIGbl1WJ",
    //     "c292LXRlc3T//////////vSMLQPlgfwCOf4QTkOhMnQxk6ra3lI+ybCMfUyanYSd",
    //     "/////////////////////wp55V2JEu8z3LhdNIIqxbq6uvpyGSGu7prq67ajVVAt",
    //     "/////////////////////7gaLStbqIBiy2pxi1D68MFUpq6sVxWBB4zdQHWHP/Tl",
    // ];

    // TODO: Re-enable this test after Celestia releases an endpoint which returns nmt proofs instead of
    // ipld.Proofs
    // #[test]
    // fn test_known_good_msg() {
    // let msg = r#"[{"Proof":{"End":1,"Nodes":[{"/":"bagao4amb5yatb7777777777773777777777777tjxe2jqsatxobgu3jqwkwsefsxscursxyaqzvvrxzv73aphwunua"},{"/":"bagao4amb5yatb77777777777777777777777776yvm54zu2vfqwyhd2nsebctxar7pxutz6uya7z3m2tzsmdtshjbm"}],"Start":0},"Shares":["c292LXRlc3QBKHsia2V5IjogInRlc3RrZXkiLCAidmFsdWUiOiAidGVzdHZhbHVlIn0AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="]}]"#;
    //     let deserialized: RpcNamespacedSharesResponse =
    //         serde_json::from_str(msg).expect("message must deserialize");

    //     let root = ns_hash_from_b64(ROW_ROOTS[0]);

    //     for row in deserialized.0.expect("shares response is not empty") {
    //         let proof: NamespaceProof<NamespacedSha2Hasher> = row.proof.into();
    //         proof
    //             .verify_range(&root, &row.shares, ROLLUP_NAMESPACE)
    //             .expect("proof should be valid");
    //     }
    // }
}
