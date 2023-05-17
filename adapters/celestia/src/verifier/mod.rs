use nmt_rs::NamespaceId;
use serde::{Deserialize, Serialize};
use sovereign_core::{
    da::{self, BlobTransactionTrait, BlockHashTrait as BlockHash, DaSpec},
    Bytes,
};

pub mod address;
pub mod proofs;

use crate::{
    pfb_from_iter,
    share_commit::recreate_commitment,
    shares::{read_varint, BlobIterator, NamespaceGroup, Share},
    types::ValidationError,
    BlobWithSender, CelestiaHeader, DataAvailabilityHeader,
};
use proofs::*;

use self::address::CelestiaAddress;

pub struct CelestiaVerifier {
    pub rollup_namespace: NamespaceId,
}

pub const PFB_NAMESPACE: NamespaceId = NamespaceId(hex_literal::hex!("0000000000000004"));
pub const PARITY_SHARES_NAMESPACE: NamespaceId = NamespaceId(hex_literal::hex!("ffffffffffffffff"));

impl BlobTransactionTrait for BlobWithSender {
    type Data = BlobIterator;
    type Address = CelestiaAddress;
    fn sender(&self) -> CelestiaAddress {
        self.sender.clone()
    }

    fn data(&self) -> Self::Data {
        self.blob.clone().into_iter()
    }
}

#[derive(Debug, PartialEq, Clone, Eq, Hash, Serialize, Deserialize)]
// Important: #[repr(transparent)] is required for safety as long as we're using
// std::mem::transmute to implement AsRef<TmHash> for tendermint::Hash
#[repr(transparent)]
pub struct TmHash(pub tendermint::Hash);

impl AsRef<[u8]> for TmHash {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

impl TmHash {
    pub fn inner(&self) -> &[u8; 32] {
        match self.0 {
            tendermint::Hash::Sha256(ref h) => h,
            tendermint::Hash::None => unreachable!("tendermint::Hash::None should not be possible"),
        }
    }
}

impl AsRef<TmHash> for tendermint::Hash {
    fn as_ref(&self) -> &TmHash {
        // Safety: #[repr(transparent)] guarantees that the memory layout of TmHash is
        // the same as tendermint::Hash, so this `transmute` is sound.
        // See https://doc.rust-lang.org/nomicon/other-reprs.html#reprtransparent
        unsafe { std::mem::transmute(self) }
    }
}

impl BlockHash for TmHash {}

pub struct CelestiaSpec;

impl DaSpec for CelestiaSpec {
    type SlotHash = TmHash;

    type BlockHeader = CelestiaHeader;

    type BlobTransaction = BlobWithSender;

    type InclusionMultiProof = Vec<EtxProof>;

    type CompletenessProof = Vec<RelevantRowProof>;

    type ChainParams = RollupParams;
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RollupParams {
    pub namespace: NamespaceId,
}

impl da::DaVerifier for CelestiaVerifier {
    type Spec = CelestiaSpec;

    type Error = ValidationError;

    fn new(params: <Self::Spec as DaSpec>::ChainParams) -> Self {
        Self {
            rollup_namespace: params.namespace,
        }
    }

    fn verify_relevant_tx_list(
        &self,
        block_header: &<Self::Spec as DaSpec>::BlockHeader,
        txs: &[<Self::Spec as DaSpec>::BlobTransaction],
        inclusion_proof: <Self::Spec as DaSpec>::InclusionMultiProof,
        completeness_proof: <Self::Spec as DaSpec>::CompletenessProof,
    ) -> Result<(), Self::Error> {
        // Validate that the provided DAH is well-formed
        block_header.validate_dah()?;

        // Check the validity and completeness of the rollup row proofs, against the DAH.
        // Extract the data from the row proofs and build a namespace_group from it
        let rollup_shares_u8 = self.verify_row_proofs(completeness_proof, &block_header.dah)?;
        if rollup_shares_u8.is_empty() {
            if txs.is_empty() {
                return Ok(());
            }
            return Err(ValidationError::MissingTx);
        }
        let namespace = NamespaceGroup::from_shares_unchecked(rollup_shares_u8);

        // Check the e-tx proofs...
        // TODO(@preston-evans98): Remove this logic if Celestia adds blob.sender metadata directly into blob
        let mut tx_iter = txs.iter();
        let square_size = block_header.dah.row_roots.len();
        for (blob, tx_proof) in namespace.blobs().zip(inclusion_proof.into_iter()) {
            // Force the row number to be monotonically increasing
            let start_offset = tx_proof.proof[0].start_offset;

            // Verify each sub-proof and flatten the shares back into a sequential array
            // First, enforce that the sub-proofs cover a contiguous range of shares
            for [l, r] in tx_proof.proof.array_windows::<2>() {
                assert_eq!(l.start_share_idx + l.shares.len(), r.start_share_idx)
            }
            let mut tx_shares = Vec::new();
            // Then, verify the sub proofs
            for sub_proof in tx_proof.proof.into_iter() {
                let row_num = sub_proof.start_share_idx / square_size;
                let root = &block_header.dah.row_roots[row_num];
                sub_proof
                    .proof
                    .verify_range(root, &sub_proof.shares, PFB_NAMESPACE)
                    .map_err(|_| ValidationError::InvalidEtxProof("invalid sub proof"))?;
                tx_shares.extend(
                    sub_proof
                        .shares
                        .into_iter()
                        .map(|share_vec| Share::new(share_vec.into())),
                )
            }

            // Next, ensure that the start_index is valid
            if !tx_shares[0].is_valid_tx_start(start_offset) {
                return Err(ValidationError::InvalidEtxProof("invalid start index"));
            }

            // Collect all of the shares data into a single array
            let trailing_shares = tx_shares[1..]
                .iter()
                .map(|share| share.data_ref().iter())
                .flatten();
            let tx_data: Vec<u8> = tx_shares[0].data_ref()[start_offset..]
                .iter()
                .chain(trailing_shares)
                .map(|x| *x)
                .collect();

            // Deserialize the pfb transaction
            let (len, len_of_len) = {
                let cursor = std::io::Cursor::new(&tx_data);
                read_varint(cursor).expect("tx must be length prefixed")
            };
            let mut cursor = std::io::Cursor::new(&tx_data[len_of_len..len as usize + len_of_len]);

            let pfb = pfb_from_iter(&mut cursor, len as usize)
                .map_err(|_| ValidationError::InvalidEtxProof("invalid pfb"))?;

            // Verify the sender and data of each blob which was sent into this namespace
            for (blob_idx, nid) in pfb.namespace_ids.iter().enumerate() {
                if nid != &self.rollup_namespace.0[..] {
                    continue;
                }
                let tx = tx_iter.next().ok_or(ValidationError::MissingTx)?;
                if tx.sender.as_ref() != pfb.signer.as_bytes() {
                    return Err(ValidationError::InvalidSigner);
                }

                let blob_ref = blob.clone();
                let blob_data: Bytes = blob.clone().data().collect();
                let tx_data: Bytes = tx.data().collect();
                assert_eq!(blob_data, tx_data);

                // Link blob commitment to e-tx commitment
                let expected_commitment =
                    recreate_commitment(square_size, blob_ref).map_err(|_| {
                        ValidationError::InvalidEtxProof("failed to recreate commitment")
                    })?;

                assert_eq!(&pfb.share_commitments[blob_idx][..], &expected_commitment);
            }
        }

        Ok(())
    }
}

impl CelestiaVerifier {
    pub fn verify_row_proofs(
        &self,
        row_proofs: Vec<RelevantRowProof>,
        dah: &DataAvailabilityHeader,
    ) -> Result<Vec<Vec<u8>>, ValidationError> {
        let mut row_proofs = row_proofs.into_iter();
        // Check the validity and completeness of the rollup share proofs
        let mut rollup_shares_u8: Vec<Vec<u8>> = Vec::new();
        for row_root in dah.row_roots.iter() {
            // TODO: short circuit this loop at the first row after the rollup namespace
            if row_root.contains(self.rollup_namespace) {
                let row_proof = row_proofs.next().ok_or(ValidationError::InvalidRowProof)?;
                row_proof
                    .proof
                    .verify_complete_namespace(row_root, &row_proof.leaves, self.rollup_namespace)
                    .expect("Proofs must be valid");

                for leaf in row_proof.leaves {
                    rollup_shares_u8.push(leaf)
                }
            }
        }
        Ok(rollup_shares_u8)
    }
}
