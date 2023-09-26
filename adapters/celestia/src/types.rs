use std::collections::HashMap;
use std::slice::Chunks;

use anyhow::{bail, ensure};
// use borsh::{BorshDeserialize, BorshSerialize};
use celestia_proto::celestia::blob::v1::MsgPayForBlobs;
use celestia_types::consts::appconsts::SHARE_SIZE;
use celestia_types::nmt::{ NamespacedHash, Nmt, NS_SIZE};
use celestia_types::{ExtendedDataSquare, NamespacedShares, ValidateBasic};
use serde::{Deserialize, Serialize};
use sov_rollup_interface::da::BlockHeaderTrait;
use sov_rollup_interface::services::da::SlotData;
use sov_rollup_interface::Bytes;

use crate::shares::NamespaceGroup;
use crate::utils::BoxError;
use crate::verifier::{ChainValidityCondition, PARITY_SHARES_NAMESPACE};
use crate::{CelestiaHeader, TxPosition};

/// Reexport the [`Namespace`] from `celestia-types`
pub use celestia_types::nmt::Namespace;

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)] // TODO: , BorshSerialize, BorshDeserialize)]
pub struct FilteredCelestiaBlock {
    pub header: CelestiaHeader,
    pub rollup_data: NamespaceGroup,
    /// A mapping from blob commitment to the PFB containing that commitment
    /// for each blob addressed to the rollup namespace
    pub relevant_pfbs: HashMap<Bytes, (MsgPayForBlobs, TxPosition)>,
    /// All rollup shares as they appear in extended data square, with proofs
    pub rollup_rows: NamespacedShares,
    /// All rows in the extended data square which contain pfb data
    pub pfb_rows: Vec<Row>,
}

impl SlotData for FilteredCelestiaBlock {
    type BlockHeader = CelestiaHeader;
    type Cond = ChainValidityCondition;

    fn hash(&self) -> [u8; 32] {
        match self.header.header.hash() {
            tendermint::Hash::Sha256(h) => h,
            tendermint::Hash::None => unreachable!("tendermint::Hash::None should not be possible"),
        }
    }

    fn header(&self) -> &Self::BlockHeader {
        &self.header
    }

    fn validity_condition(&self) -> ChainValidityCondition {
        ChainValidityCondition {
            prev_hash: *self.header().prev_hash().inner(),
            block_hash: self.hash(),
        }
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

#[derive(Debug, thiserror::Error)]
pub enum ValidationError {
    #[error("Missing data hash in header")]
    MissingDataHash,

    #[error("Data root hash doesn't match computed one")]
    InvalidDataRoot,

    #[error("Invalid etx proof: {0}")]
    InvalidEtxProof(&'static str),

    #[error("Transaction missing")]
    MissingTx,

    #[error("Invalid row proof")]
    InvalidRowProof,

    #[error("Invalid signer")]
    InvalidSigner,

    #[error("Incomplete data")]
    IncompleteData,

    #[error(transparent)]
    DahValidation(#[from] celestia_types::ValidationError),
}

impl CelestiaHeader {
    pub fn validate_dah(&self) -> Result<(), ValidationError> {
        self.dah.validate_basic()?;
        let data_hash = self
            .header
            .data_hash
            .as_ref()
            .ok_or(ValidationError::MissingDataHash)?;
        if self.dah.hash != data_hash.0 {
            return Err(ValidationError::InvalidDataRoot);
        }
        Ok(())
    }
}

pub trait ExtendedDataSquareExt {
    fn square_size(&self) -> Result<usize, BoxError>;

    fn rows(&self) -> Result<Chunks<'_, Vec<u8>>, BoxError>;

    fn validate(&self) -> Result<(), BoxError>;
}

impl ExtendedDataSquareExt for ExtendedDataSquare {
    fn square_size(&self) -> Result<usize, BoxError> {
        let len = self.data_square.len();
        let square_size = (len as f64).sqrt() as usize;
        ensure!(
            square_size * square_size == len,
            "eds size {} is not a perfect square",
            len
        );
        Ok(square_size)
    }

    fn rows(&self) -> Result<Chunks<'_, Vec<u8>>, BoxError> {
        let square_size = self.square_size()?;
        Ok(self.data_square.chunks(square_size))
    }

    fn validate(&self) -> Result<(), BoxError> {
        let len = self.square_size()?;
        ensure!(len * len == self.data_square.len(), "Invalid square size");

        if let Some(share) = self
            .rows()
            .expect("after first check this must succeed")
            .flatten()
            .find(|shares| shares.len() != SHARE_SIZE)
        {
            bail!("Invalid share size: {}", share.len())
        }
        Ok(())
    }
}

#[derive(Debug, PartialEq, Clone, serde::Serialize, serde::Deserialize)] // TODO: , BorshSerialize, BorshDeserialize)]
pub struct Row {
    pub shares: Vec<Vec<u8>>,
    pub root: NamespacedHash,
}

impl Row {
    pub fn merklized(&self) -> Nmt {
        let mut nmt = Nmt::new();
        for (idx, share) in self.shares.iter().enumerate() {
            // Shares in the two left-hand quadrants are prefixed with their namespace, while parity
            // shares (in the right-hand) quadrants should always be treated as PARITY_SHARES_NAMESPACE
            let namespace = if idx < self.shares.len() / 2 {
                share_namespace_unchecked(share)
            } else {
                PARITY_SHARES_NAMESPACE
            };
            nmt.push_leaf(share.as_ref(), *namespace)
                .expect("shares are pushed in order");
        }
        nmt
    }
}

/// get namespace from a share without verifying if it's a correct namespace
/// (version 0 or parity ns).
fn share_namespace_unchecked(share: &[u8]) -> Namespace {
    nmt_rs::NamespaceId(
        share[..NS_SIZE]
            .try_into()
            .expect("must succeed for correct size"),
    )
    .into()
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
