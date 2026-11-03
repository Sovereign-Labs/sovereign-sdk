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

#[cfg(test)]
mod tests {
    use celestia_fibre::{BlobConfig, DEFAULT_PROTOCOL_PARAMS};

    use super::*;
    use crate::verifier::{
        row_size_for, FIBRE_HEADER_LEN, FIBRE_MAX_BLOB_SIZE, FIBRE_N, FIBRE_ROW_ALIGN,
    };

    #[test]
    fn constants_match_lumina_defaults() {
        assert_eq!(DEFAULT_PROTOCOL_PARAMS.rows.get(), FIBRE_K);
        assert_eq!(
            DEFAULT_PROTOCOL_PARAMS.total_rows() - DEFAULT_PROTOCOL_PARAMS.rows.get(),
            FIBRE_N
        );
        assert_eq!(DEFAULT_PROTOCOL_PARAMS.min_row_size, FIBRE_ROW_ALIGN);
        assert_eq!(DEFAULT_PROTOCOL_PARAMS.max_blob_size, FIBRE_MAX_BLOB_SIZE);
        let data_len = 20_000;
        assert_eq!(
            BlobConfig::v0().row_size(data_len),
            row_size_for(data_len).unwrap()
        );
        let _ = FIBRE_HEADER_LEN;
    }

    #[tokio::test]
    async fn rows_proof_matches_commitment() {
        let data: Vec<u8> = (0..20_000).map(|i| (i * 13) as u8).collect();
        let encoded = encode(&data).await.unwrap();
        for prefix in [0, 1000, data.len()] {
            let proof = build_rows_proof(&encoded, &data, prefix).unwrap();
            verify_fibre_prefix(
                &encoded.id().commitment(),
                &proof,
                &data[..prefix],
                data.len(),
            )
            .unwrap();
        }
    }
}
