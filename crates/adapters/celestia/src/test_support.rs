use celestia_types::consts::appconsts;
use celestia_types::namespace_data::NamespaceData;
use celestia_types::nmt::Namespace;
use celestia_types::state::AccAddress;
use celestia_types::{Blob, DataAvailabilityHeader, ExtendedDataSquare};
use tendermint::Hash;

use crate::celestia::{CelestiaHeader, CompactHeader, ProtobufHash};
use crate::types::{FilteredCelestiaBlock, NamespaceRelevantData, APP_VERSION};
use crate::verifier::proofs::BlobProof;
use crate::verifier::RollupParams;

pub(crate) const NS_LOW_A: Namespace = Namespace::const_v0([0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
pub(crate) const NS_LOW_B: Namespace = Namespace::const_v0([0, 0, 0, 0, 0, 0, 0, 0, 0, 2]);
pub(crate) const NS_BATCH: Namespace = Namespace::const_v0([0, 0, 0, 0, 0, 0, 0, 0, 0, 3]);
pub(crate) const NS_HIGH_A: Namespace = Namespace::const_v0([0, 0, 0, 0, 0, 0, 0, 0, 0, 4]);
pub(crate) const NS_HIGH_B: Namespace = Namespace::const_v0([0, 0, 0, 0, 0, 0, 0, 0, 0, 5]);
pub(crate) const NS_PROOF: Namespace = Namespace::const_v0([0, 0, 0, 0, 0, 0, 0, 0, 0, 6]);

pub(crate) const PREFIX_NAMESPACES: [Namespace; 2] = [NS_LOW_A, NS_LOW_B];
pub(crate) const SUFFIX_NAMESPACES: [Namespace; 2] = [NS_HIGH_A, NS_HIGH_B];

pub(crate) fn rollup_params() -> RollupParams {
    RollupParams {
        rollup_batch_namespace: NS_BATCH,
        rollup_proof_namespace: NS_PROOF,
    }
}

pub(crate) fn append_segmented_blobs(
    shares: &mut Vec<Vec<u8>>,
    segments: &[usize],
    namespaces: &[Namespace],
    signer: Option<AccAddress>,
    seed: u8,
) {
    assert!(segments.len() <= namespaces.len());
    for (idx, share_count) in segments.iter().enumerate() {
        shares.extend(make_blob_shares(
            namespaces[idx],
            *share_count,
            signer,
            seed.wrapping_add(idx as u8),
        ));
    }
}

pub(crate) fn make_blob_shares(
    namespace: Namespace,
    share_count: usize,
    signer: Option<AccAddress>,
    seed: u8,
) -> Vec<Vec<u8>> {
    assert!(share_count > 0, "share count must be positive");
    let payload_len = payload_len_for_share_count(share_count, signer.is_some());
    let payload = vec![seed; payload_len];
    let blob = Blob::new(namespace, payload, signer, APP_VERSION).expect("blob creation failed");
    let shares = blob.to_shares().expect("blob to shares failed");
    assert_eq!(shares.len(), share_count, "unexpected share count");
    shares.into_iter().map(|share| share.to_vec()).collect()
}

pub(crate) fn payload_len_for_share_count(share_count: usize, has_signer: bool) -> usize {
    let first_share_content = if has_signer {
        appconsts::FIRST_SPARSE_SHARE_CONTENT_SIZE
            .checked_sub(appconsts::SIGNER_SIZE)
            .expect("signer size should fit into first share content size")
    } else {
        appconsts::FIRST_SPARSE_SHARE_CONTENT_SIZE
    };

    if share_count <= 1 {
        return first_share_content.saturating_sub(1).max(1);
    }

    first_share_content + (share_count - 2) * appconsts::CONTINUATION_SPARSE_SHARE_CONTENT_SIZE + 1
}

pub(crate) fn signer_for_index(seed: u8, idx: usize) -> AccAddress {
    let byte = seed.wrapping_add(idx as u8).wrapping_add(1);
    let bytes = [byte; 20];
    AccAddress::try_from(&bytes[..]).expect("valid signer bytes")
}

pub(crate) fn namespace_rows(
    eds: &ExtendedDataSquare,
    dah: &DataAvailabilityHeader,
    namespace: Namespace,
) -> Vec<celestia_types::row_namespace_data::RowNamespaceData> {
    eds.get_namespace_data(namespace, dah, 1)
        .expect("namespace data fetch failed")
        .into_iter()
        .map(|(_, row)| row)
        .collect()
}

pub(crate) fn build_block_from_ods(ods_shares: Vec<Vec<u8>>) -> FilteredCelestiaBlock {
    let eds = ExtendedDataSquare::from_ods(ods_shares, APP_VERSION).expect("EDS creation failed");
    let dah = DataAvailabilityHeader::from_eds(&eds);
    let batch_rows = namespace_rows(&eds, &dah, NS_BATCH);
    let proof_rows = namespace_rows(&eds, &dah, NS_PROOF);

    FilteredCelestiaBlock {
        header: build_header(dah),
        rollup_batch_data: NamespaceRelevantData::new(NS_BATCH, NamespaceData::new(batch_rows)),
        rollup_proof_data: NamespaceRelevantData::new(NS_PROOF, NamespaceData::new(proof_rows)),
    }
}

pub(crate) fn build_header(dah: DataAvailabilityHeader) -> CelestiaHeader {
    let data_hash = match dah.hash() {
        Hash::Sha256(bytes) => Some(ProtobufHash(bytes)),
        Hash::None => None,
    };

    let compact = CompactHeader {
        version: Vec::new(),
        chain_id: Vec::new(),
        height: Vec::new(),
        time: Vec::new(),
        last_block_id: Vec::new(),
        last_commit_hash: Vec::new(),
        data_hash,
        validators_hash: Vec::new(),
        next_validators_hash: Vec::new(),
        consensus_hash: Vec::new(),
        app_hash: Vec::new(),
        last_results_hash: Vec::new(),
        evidence_hash: Vec::new(),
        proposer_address: Vec::new(),
    };

    CelestiaHeader::new(dah, compact)
}

pub(crate) fn assert_subproof_start_indices_align(
    block: &FilteredCelestiaBlock,
    namespace_name: &str,
    inclusion_proof: &[BlobProof],
) {
    let row_len = block.header.row_length();
    for (blob_idx, blob_proof) in inclusion_proof.iter().enumerate() {
        for (range_idx, range_proof) in blob_proof.range_proofs.iter().enumerate() {
            let row = block
                .header
                .calculate_row_number_for_share(range_proof.start_share_idx);
            let expected = row
                .checked_mul(row_len)
                .and_then(|row_start| row_start.checked_add(range_proof.proof.start_idx() as usize))
                .expect("overflow while calculating expected share index");
            assert_eq!(
                range_proof.start_share_idx, expected,
                "{namespace_name} proof index mismatch for blob #{blob_idx} range #{range_idx}: expected {expected}, actual {}",
                range_proof.start_share_idx
            );
        }
    }
}
