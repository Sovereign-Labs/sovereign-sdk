// use borsh::{BorshDeserialize, BorshSerialize};
use celestia_types::nmt::NamespaceProof;
use celestia_types::NamespacedShares;
use serde::{Deserialize, Serialize};

use super::CelestiaSpec;
use crate::types::FilteredCelestiaBlock;

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)] // TODO:, BorshDeserialize, BorshSerialize)]
pub struct EtxProof {
    pub proof: Vec<EtxRangeProof>,
}

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)] // TODO:, BorshDeserialize, BorshSerialize)]
pub struct EtxRangeProof {
    pub shares: Vec<Vec<u8>>,
    pub proof: NamespaceProof,
    pub start_share_idx: usize,
    pub start_offset: usize,
}

#[derive(Debug, PartialEq, Clone)]
pub struct CompletenessProof(pub NamespacedShares);

impl CompletenessProof {
    pub fn from_filtered_block(block: &FilteredCelestiaBlock) -> Self {
        Self(block.rollup_rows.clone())
    }
}

pub struct CorrectnessProof(pub Vec<EtxProof>);

impl CorrectnessProof {
    pub fn for_block(
        block: &FilteredCelestiaBlock,
        blobs: &[<CelestiaSpec as sov_rollup_interface::da::DaSpec>::BlobTransaction],
    ) -> Self {
        let mut needed_tx_shares = Vec::new();

        // Extract (and clone) the position of each transaction
        for tx in blobs.iter() {
            let (_, position) = block
                .relevant_pfbs
                .get(tx.hash.as_slice())
                .expect("commitment must exist in map");
            needed_tx_shares.push(position.clone());
        }

        let mut needed_tx_shares = needed_tx_shares.into_iter().peekable();
        let mut current_tx_proof: EtxProof = EtxProof { proof: Vec::new() };
        let mut tx_proofs: Vec<EtxProof> = Vec::with_capacity(blobs.len());

        for (row_idx, row) in block.pfb_rows.iter().enumerate() {
            let mut nmt = row.merklized();
            while let Some(next_needed_share) = needed_tx_shares.peek_mut() {
                // If the next needed share falls in this row
                let row_start_idx = block.square_size() * row_idx;
                let start_column_number = next_needed_share.share_range.start - row_start_idx;
                if start_column_number < block.square_size() {
                    let end_column_number = next_needed_share.share_range.end - row_start_idx;
                    if end_column_number <= block.square_size() {
                        let (shares, proof) =
                            nmt.get_range_with_proof(start_column_number..end_column_number);

                        current_tx_proof.proof.push(EtxRangeProof {
                            shares,
                            proof: proof.into(),
                            start_offset: next_needed_share.start_offset,
                            start_share_idx: next_needed_share.share_range.start,
                        });
                        tx_proofs.push(current_tx_proof);
                        current_tx_proof = EtxProof { proof: Vec::new() };
                        let _ = needed_tx_shares.next();
                    } else {
                        let (shares, proof) =
                            nmt.get_range_with_proof(start_column_number..block.square_size());

                        current_tx_proof.proof.push(EtxRangeProof {
                            shares,
                            proof: proof.into(),
                            start_offset: next_needed_share.start_offset,
                            start_share_idx: next_needed_share.share_range.start,
                        });
                        next_needed_share.share_range.start = block.square_size() * (row_idx + 1);
                        next_needed_share.start_offset = 0;

                        break;
                    }
                } else {
                    break;
                }
            }
        }
        Self(tx_proofs)
    }
}
