use std::collections::HashMap;
use std::slice::Chunks;

use anyhow::{bail, ensure};
// use borsh::{BorshDeserialize, BorshSerialize};
use celestia_proto::celestia::blob::v1::MsgPayForBlobs;
use celestia_types::consts::appconsts::SHARE_SIZE;
/// Reexport the [`Namespace`] from `celestia-types`
pub use celestia_types::nmt::Namespace;
use celestia_types::nmt::{NamespacedHash, Nmt, NS_SIZE};
use celestia_types::{
    DataAvailabilityHeader, ExtendedDataSquare, ExtendedHeader, NamespacedShares, ValidateBasic,
};
use serde::{Deserialize, Serialize};
use sov_rollup_interface::da::BlockHeaderTrait;
use sov_rollup_interface::services::da::SlotData;
use sov_rollup_interface::Bytes;
use tracing::debug;

use crate::shares::NamespaceGroup;
use crate::utils::BoxError;
use crate::verifier::{ChainValidityCondition, PARITY_SHARES_NAMESPACE, PFB_NAMESPACE};
use crate::{parse_pfb_namespace, CelestiaHeader, TxPosition};

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
    pub fn new(
        rollup_ns: Namespace,
        header: ExtendedHeader,
        rollup_rows: NamespacedShares,
        etx_rows: NamespacedShares,
        data_square: ExtendedDataSquare,
    ) -> Result<Self, BoxError> {
        // validate the extended data square
        data_square.validate()?;

        let rollup_data = NamespaceGroup::from(&rollup_rows);
        let tx_data = NamespaceGroup::from(&etx_rows);

        // Parse out all of the rows containing etxs
        debug!("Parsing namespaces...");
        let pfb_rows =
            get_rows_containing_namespace(PFB_NAMESPACE, &header.dah, data_square.rows()?)?;

        // Parse out the pfds and store them for later retrieval
        debug!("Decoding pfb protobufs...");
        let pfbs = parse_pfb_namespace(tx_data)?;
        let mut pfb_map = HashMap::new();
        for tx in pfbs {
            for (idx, nid) in tx.0.namespaces.iter().enumerate() {
                if nid == rollup_ns.as_bytes() {
                    // TODO: Retool this map to avoid cloning txs
                    pfb_map.insert(tx.0.share_commitments[idx].clone().into(), tx.clone());
                }
            }
        }

        Ok(FilteredCelestiaBlock {
            header: CelestiaHeader::new(header.dah, header.header.into()),
            rollup_data,
            relevant_pfbs: pfb_map,
            rollup_rows,
            pfb_rows,
        })
    }

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
        if self.dah.hash().as_ref() != data_hash.0 {
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

fn get_rows_containing_namespace<'a>(
    nid: Namespace,
    dah: &'a DataAvailabilityHeader,
    data_square_rows: impl Iterator<Item = &'a [Vec<u8>]>,
) -> Result<Vec<Row>, BoxError> {
    let mut output = vec![];

    for (row, root) in data_square_rows.zip(dah.row_roots.iter()) {
        if root.contains(*nid) {
            output.push(Row {
                shares: row.to_vec(),
                root: root.clone(),
            })
        }
    }
    Ok(output)
}

#[cfg(test)]
pub mod tests {
    use celestia_types::nmt::Namespace;
    use celestia_types::{ExtendedDataSquare, ExtendedHeader, NamespacedShares};

    use super::FilteredCelestiaBlock;
    use crate::verifier::PFB_NAMESPACE;

    pub const ROLLUP_NAMESPACE: Namespace = Namespace::const_v0(*b"\0\0sov-test");

    pub mod with_rollup_data {
        use super::*;

        pub const HEADER_JSON: &str =
            include_str!("../test_data/block_with_rollup_data/header.json");
        pub const ROLLUP_ROWS_JSON: &str =
            include_str!("../test_data/block_with_rollup_data/rollup_rows.json");
        pub const ETX_ROWS_JSON: &str =
            include_str!("../test_data/block_with_rollup_data/etx_rows.json");
        pub const EDS_JSON: &str = include_str!("../test_data/block_with_rollup_data/eds.json");

        pub fn filtered_block() -> FilteredCelestiaBlock {
            filtered_block_from_jsons(
                ROLLUP_NAMESPACE,
                HEADER_JSON,
                ROLLUP_ROWS_JSON,
                ETX_ROWS_JSON,
                EDS_JSON,
            )
        }
    }

    pub mod without_rollup_data {
        use super::*;

        pub const HEADER_JSON: &str =
            include_str!("../test_data/block_without_rollup_data/header.json");
        pub const ROLLUP_ROWS_JSON: &str =
            include_str!("../test_data/block_without_rollup_data/rollup_rows.json");
        pub const ETX_ROWS_JSON: &str =
            include_str!("../test_data/block_without_rollup_data/etx_rows.json");
        pub const EDS_JSON: &str = include_str!("../test_data/block_without_rollup_data/eds.json");

        pub fn filtered_block() -> FilteredCelestiaBlock {
            filtered_block_from_jsons(
                ROLLUP_NAMESPACE,
                HEADER_JSON,
                ROLLUP_ROWS_JSON,
                ETX_ROWS_JSON,
                EDS_JSON,
            )
        }
    }

    fn filtered_block_from_jsons(
        ns: Namespace,
        header: &str,
        rollup_rows: &str,
        etx_rows: &str,
        eds: &str,
    ) -> FilteredCelestiaBlock {
        let header: ExtendedHeader = serde_json::from_str(header).unwrap();
        let rollup_rows: NamespacedShares = serde_json::from_str(rollup_rows).unwrap();
        let etx_rows: NamespacedShares = serde_json::from_str(etx_rows).unwrap();
        let eds: ExtendedDataSquare = serde_json::from_str(eds).unwrap();

        FilteredCelestiaBlock::new(ns, header, rollup_rows, etx_rows, eds).unwrap()
    }

    #[test]
    fn filtered_block_with_rollup_data() {
        let block = with_rollup_data::filtered_block();

        // valid dah
        block.header.validate_dah().unwrap();

        // single rollup share
        assert_eq!(block.rollup_data.shares().len(), 1);
        assert_eq!(block.rollup_rows.rows.len(), 1);
        assert_eq!(block.rollup_rows.rows[0].shares.len(), 1);
        assert!(block.rollup_rows.rows[0].proof.is_of_presence());

        // 3 pfbs at all but only one belongs to rollup
        assert_eq!(block.pfb_rows.len(), 1);
        let pfbs_count = block.pfb_rows[0]
            .shares
            .iter()
            .filter(|share| share.starts_with(PFB_NAMESPACE.as_ref()))
            .count();
        assert_eq!(pfbs_count, 3);
        assert_eq!(block.relevant_pfbs.len(), 1);
    }

    #[test]
    fn filtered_block_without_rollup_data() {
        let block = without_rollup_data::filtered_block();

        // valid dah
        block.header.validate_dah().unwrap();

        // no rollup shares
        assert_eq!(block.rollup_data.shares().len(), 0);
        // we still get single row, but with absence proof and no shares
        assert_eq!(block.rollup_rows.rows.len(), 1);
        assert_eq!(block.rollup_rows.rows[0].shares.len(), 0);
        assert!(block.rollup_rows.rows[0].proof.is_of_absence());

        // 2 pfbs at all and no relevant
        assert_eq!(block.pfb_rows.len(), 1);
        let pfbs_count = block.pfb_rows[0]
            .shares
            .iter()
            .filter(|share| share.starts_with(PFB_NAMESPACE.as_ref()))
            .count();
        assert_eq!(pfbs_count, 2);
        assert_eq!(block.relevant_pfbs.len(), 0);
    }
}
