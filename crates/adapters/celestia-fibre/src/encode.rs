use celestia_fibre::{BlobConfig, EncodedBlob};

use crate::proof::FibreRowsProof;
use crate::verifier::{rows_needed_for_prefix, verify_fibre_prefix, FIBRE_HEADER_LEN, FIBRE_K};

pub async fn encode(data: &[u8]) -> anyhow::Result<EncodedBlob> {
    let owned = data.to_vec();
    tokio::task::spawn_blocking(move || {
        EncodedBlob::new(&owned, BlobConfig::v0()).map_err(|error| anyhow::anyhow!(error))
    })
    .await?
}

pub fn build_rows_proof(
    encoded: &EncodedBlob,
    data: &[u8],
    prefix_len: usize,
) -> anyhow::Result<FibreRowsProof> {
    let row_size = encoded.row_size();
    let prefix_rows = rows_needed_for_prefix(prefix_len, row_size);
    anyhow::ensure!(
        (1..=FIBRE_K).contains(&prefix_rows),
        "Fibre prefix of {prefix_rows} rows is out of range"
    );
    let mut rows = Vec::with_capacity(FIBRE_HEADER_LEN + data.len());
    rows.push(0);
    rows.extend_from_slice(&u32::try_from(data.len())?.to_be_bytes());
    rows.extend_from_slice(data);
    rows.resize(prefix_rows * row_size, 0);

    let index = prefix_rows - 1;
    let inclusion = encoded.row(index).map_err(|error| anyhow::anyhow!(error))?;
    let rlc_root = inclusion.rlc_root;
    let siblings = inclusion
        .row_proof
        .into_iter()
        .enumerate()
        .filter(|(level, _)| index & (1usize << level) == 0)
        .map(|(_, sibling)| sibling)
        .collect();
    let proof = FibreRowsProof {
        row_size: u32::try_from(row_size)?,
        rlc_root,
        rows,
        siblings,
    };
    debug_assert!(
        verify_fibre_prefix(
            &encoded.id().commitment(),
            &proof,
            &data[..prefix_len],
            data.len()
        )
        .is_ok(),
        "rows proof does not authenticate the encoded blob"
    );
    Ok(proof)
}
