use crate::test_support::{
    build_block_from_ods, make_blob_shares, rollup_params, signer_for_index, NS_BATCH, NS_HIGH_A,
    NS_LOW_A,
};
use crate::types::FilteredCelestiaBlock;
use crate::verifier::RollupParams;

pub(super) use crate::test_support::synthetic_noncanonical_multirow_absence_fixture;

/// Canonical single-row absence fixture with one candidate row and no `NS_BATCH` shares.
///
/// ```text
/// ODS 4x4
/// row0: L L L H
/// row1: H H H H
/// row2: H H H H
/// row3: H H H H
///
/// row-major: L L L H | H H H H | H H H H | H H H H
/// verdict: canonical
/// reason: namespaces never decrease in row-major order
/// ```
pub(super) fn canonical_single_row_absence_fixture() -> (FilteredCelestiaBlock, RollupParams) {
    let ods_width = 4usize;
    let mut ods_shares = Vec::with_capacity(ods_width * ods_width);

    // Row 0: low + high, no target shares (single candidate row with absence).
    ods_shares.extend(make_blob_shares(NS_LOW_A, 3, None, 0x41));
    ods_shares.extend(make_blob_shares(NS_HIGH_A, 1, None, 0x42));

    // Rows 1-3: high only, non-candidate rows for NS_BATCH.
    ods_shares.extend(make_blob_shares(NS_HIGH_A, ods_width * 3, None, 0x43));

    let block = build_block_from_ods(ods_shares);
    let batch_rows = block.rollup_batch_data.data.rows();

    assert_eq!(
        batch_rows.len(),
        1,
        "synthetic fixture must contain exactly one candidate row"
    );
    assert!(
        batch_rows[0].shares.is_empty(),
        "synthetic fixture must represent absence in the only candidate row"
    );

    (block, rollup_params())
}

/// Synthetic mixed-presence fixture with one absence candidate row and one real `NS_BATCH` row.
///
/// ```text
/// ODS 4x4
/// row0: L L L H
/// row1: L B H H
/// row2: H H H H
/// row3: H H H H
///
/// row-major: L L L H | L B H H | H H H H | H H H H
/// verdict: non-canonical
/// reason: row-major order decreases H -> L across the row0 -> row1 boundary
/// ```
///
/// This is a synthetic adversarial test shape used to exercise verifier behavior, not an
/// honest-proposer Celestia ODS layout.
pub(super) fn synthetic_noncanonical_multirow_mixed_presence_fixture(
) -> (FilteredCelestiaBlock, RollupParams) {
    let ods_width = 4usize;
    let mut ods_shares = Vec::with_capacity(ods_width * ods_width);

    // Row 0: low + high, no target shares (candidate row with absence).
    ods_shares.extend(make_blob_shares(NS_LOW_A, 3, None, 0x11));
    ods_shares.extend(make_blob_shares(NS_HIGH_A, 1, None, 0x12));

    // Row 1: low + target + high (candidate row with real target shares).
    ods_shares.extend(make_blob_shares(NS_LOW_A, 1, None, 0x21));
    ods_shares.extend(make_blob_shares(
        NS_BATCH,
        1,
        Some(signer_for_index(0x22, 0)),
        0x23,
    ));
    ods_shares.extend(make_blob_shares(NS_HIGH_A, 2, None, 0x24));

    // Rows 2-3: high only, non-candidate rows for NS_BATCH.
    ods_shares.extend(make_blob_shares(NS_HIGH_A, ods_width * 2, None, 0x31));

    let block = build_block_from_ods(ods_shares);
    let batch_rows = block.rollup_batch_data.data.rows();

    assert!(
        batch_rows.len() > 1,
        "synthetic fixture must contain multiple candidate rows"
    );
    assert!(
        batch_rows.iter().any(|row| row.shares.is_empty()),
        "synthetic fixture must contain at least one absence candidate row"
    );
    assert!(
        batch_rows.iter().any(|row| !row.shares.is_empty()),
        "synthetic fixture must contain at least one presence candidate row"
    );

    (block, rollup_params())
}
