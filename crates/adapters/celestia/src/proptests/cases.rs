use celestia_types::nmt::NS_SIZE;
use proptest::prelude::*;
use proptest::strategy::BoxedStrategy;

use crate::test_support::{
    append_segmented_blobs, build_block_from_ods, make_blob_shares, signer_for_index, NS_BATCH,
    NS_HIGH_A, NS_LOW_A, PREFIX_NAMESPACES, SUFFIX_NAMESPACES,
};
use crate::types::FilteredCelestiaBlock;

#[derive(Debug, Clone)]
pub(crate) struct MultiRowBatchCase {
    pub(crate) ods_width: usize,
    pub(crate) prefix_rows: usize,
    pub(crate) batch_rows_full: usize,
    pub(crate) suffix_rows: usize,
    pub(crate) batch_blob_shares: Vec<usize>,
    pub(crate) prefix_segments: Vec<usize>,
    pub(crate) suffix_segments: Vec<usize>,
    pub(crate) seed: u8,
}

impl MultiRowBatchCase {
    pub(crate) fn prefix_total(&self) -> usize {
        self.prefix_rows * self.ods_width
    }

    pub(crate) fn batch_total(&self) -> usize {
        self.batch_rows_full * self.ods_width
    }

    pub(crate) fn suffix_total(&self) -> usize {
        self.suffix_rows * self.ods_width
    }

    pub(crate) fn build_block(&self) -> FilteredCelestiaBlock {
        build_block_from_ods(self.build_ods_shares())
    }

    pub(crate) fn build_ods_shares(&self) -> Vec<Vec<u8>> {
        let prefix_total = self.prefix_total();
        let batch_total = self.batch_total();
        let suffix_total = self.suffix_total();
        let total_shares = self.ods_width * self.ods_width;

        assert_eq!(prefix_total, self.prefix_segments.iter().sum::<usize>());
        assert_eq!(batch_total, self.batch_blob_shares.iter().sum::<usize>());
        assert_eq!(suffix_total, self.suffix_segments.iter().sum::<usize>());
        assert_eq!(prefix_total + batch_total + suffix_total, total_shares);

        let mut shares = Vec::with_capacity(total_shares);
        append_segmented_blobs(
            &mut shares,
            &self.prefix_segments,
            &PREFIX_NAMESPACES,
            None,
            self.seed,
        );
        for (idx, share_count) in self.batch_blob_shares.iter().enumerate() {
            shares.extend(make_blob_shares(
                NS_BATCH,
                *share_count,
                Some(signer_for_index(self.seed, idx)),
                self.seed.wrapping_add(idx as u8),
            ));
        }
        append_segmented_blobs(
            &mut shares,
            &self.suffix_segments,
            &SUFFIX_NAMESPACES,
            None,
            self.seed.wrapping_add(0x80),
        );
        assert_eq!(shares.len(), total_shares);
        assert_canonical_ods_namespaces(&shares);

        shares
    }
}

#[derive(Debug, Clone)]
pub(crate) struct MidRowBatchCase {
    pub(crate) ods_width: usize,
    pub(crate) prefix_rows: usize,
    pub(crate) prefix_len: usize,
    pub(crate) batch_rows_full: usize,
    pub(crate) last_batch_len: usize,
    pub(crate) suffix_rows: usize,
    pub(crate) seed: u8,
}

impl MidRowBatchCase {
    pub(crate) fn prefix_total(&self) -> usize {
        self.prefix_rows * self.ods_width + self.prefix_len
    }

    pub(crate) fn batch_total(&self) -> usize {
        (self.ods_width - self.prefix_len)
            + self.batch_rows_full * self.ods_width
            + self.last_batch_len
    }

    pub(crate) fn suffix_total(&self) -> usize {
        self.suffix_rows * self.ods_width + (self.ods_width - self.last_batch_len)
    }

    pub(crate) fn build_block(&self) -> FilteredCelestiaBlock {
        let prefix_total = self.prefix_total();
        let batch_total = self.batch_total();
        let suffix_total = self.suffix_total();
        let total_shares = self.ods_width * self.ods_width;

        assert_eq!(prefix_total + batch_total + suffix_total, total_shares);

        let mut shares = Vec::with_capacity(total_shares);
        append_segmented_blobs(
            &mut shares,
            &[prefix_total],
            &PREFIX_NAMESPACES,
            None,
            self.seed,
        );
        shares.extend(make_blob_shares(
            NS_BATCH,
            batch_total,
            Some(signer_for_index(self.seed, 0)),
            self.seed,
        ));
        append_segmented_blobs(
            &mut shares,
            &[suffix_total],
            &SUFFIX_NAMESPACES,
            None,
            self.seed.wrapping_add(0x80),
        );
        assert_eq!(shares.len(), total_shares);

        build_canonical_block_from_ods(shares)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct SingleRowMidRowCase {
    ods_width: usize,
    prefix_rows: usize,
    prefix_len: usize,
    batch_len: usize,
    suffix_rows: usize,
    seed: u8,
}

impl SingleRowMidRowCase {
    pub fn build_block(&self) -> FilteredCelestiaBlock {
        let suffix_len = self
            .ods_width
            .checked_sub(self.prefix_len)
            .and_then(|remaining| remaining.checked_sub(self.batch_len))
            .expect("invalid case: prefix_len + batch_len must be < ods_width");
        let prefix_total = self.prefix_rows * self.ods_width + self.prefix_len;
        let suffix_total = self.suffix_rows * self.ods_width + suffix_len;
        let total_shares = self.ods_width * self.ods_width;

        assert!(
            suffix_len > 0,
            "single-row mid-row case requires suffix in row"
        );
        assert_eq!(prefix_total + self.batch_len + suffix_total, total_shares);

        let mut shares = Vec::with_capacity(total_shares);
        if prefix_total > 0 {
            append_segmented_blobs(
                &mut shares,
                &[prefix_total],
                &PREFIX_NAMESPACES,
                None,
                self.seed,
            );
        }
        shares.extend(make_blob_shares(
            NS_BATCH,
            self.batch_len,
            Some(signer_for_index(self.seed, 0)),
            self.seed,
        ));
        if suffix_total > 0 {
            append_segmented_blobs(
                &mut shares,
                &[suffix_total],
                &SUFFIX_NAMESPACES,
                None,
                self.seed.wrapping_add(0x80),
            );
        }
        assert_eq!(shares.len(), total_shares);

        build_canonical_block_from_ods(shares)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct SingleRowAbsenceCase {
    ods_width: usize,
    candidate_low_len: usize,
    seed: u8,
}

impl SingleRowAbsenceCase {
    pub fn build_block(&self) -> FilteredCelestiaBlock {
        let suffix_rows = self
            .ods_width
            .checked_sub(1)
            .expect("single-row absence requires exactly one candidate row");
        let candidate_high_len = self
            .ods_width
            .checked_sub(self.candidate_low_len)
            .expect("candidate low_len must fit within row width");
        let total_shares = self.ods_width * self.ods_width;
        assert!(self.candidate_low_len > 0);
        assert!(candidate_high_len > 0);

        let mut shares = Vec::with_capacity(total_shares);
        shares.extend(make_blob_shares(
            NS_LOW_A,
            self.candidate_low_len,
            None,
            self.seed.wrapping_add(0x20),
        ));
        shares.extend(make_blob_shares(
            NS_HIGH_A,
            candidate_high_len,
            None,
            self.seed.wrapping_add(0x21),
        ));
        for row_idx in 0..suffix_rows {
            shares.extend(make_blob_shares(
                NS_HIGH_A,
                self.ods_width,
                None,
                self.seed.wrapping_add(0x40).wrapping_add(row_idx as u8),
            ));
        }

        assert_eq!(shares.len(), total_shares);
        build_canonical_block_from_ods(shares)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ZeroRowAbsenceCase {
    ods_width: usize,
    seed: u8,
}

impl ZeroRowAbsenceCase {
    pub fn build_block(&self) -> FilteredCelestiaBlock {
        let total_shares = self.ods_width * self.ods_width;
        let mut shares = Vec::with_capacity(total_shares);
        // Fill all rows with NS_HIGH_A (ns=4 > NS_BATCH=3), so no row contains NS_BATCH.
        for row_idx in 0..self.ods_width {
            shares.extend(make_blob_shares(
                NS_HIGH_A,
                self.ods_width,
                None,
                self.seed.wrapping_add(row_idx as u8),
            ));
        }
        assert_eq!(shares.len(), total_shares);
        build_canonical_block_from_ods(shares)
    }
}

pub fn multi_row_case_strategy() -> BoxedStrategy<MultiRowBatchCase> {
    prop_oneof![Just(4usize), Just(8usize), Just(32usize)]
        .prop_flat_map(|ods_width| {
            (0..ods_width).prop_flat_map(move |prefix_rows| {
                let remaining = ods_width - prefix_rows - 1;
                (0..=remaining, any::<u8>()).prop_map(move |(suffix_rows, seed)| {
                    let batch_rows = ods_width - prefix_rows - suffix_rows;
                    let prefix_total = prefix_rows * ods_width;
                    let batch_total = batch_rows * ods_width;
                    let suffix_total = suffix_rows * ods_width;

                    MultiRowBatchCase {
                        ods_width,
                        prefix_rows,
                        batch_rows_full: batch_rows,
                        suffix_rows,
                        batch_blob_shares: split_total_across_segments(
                            batch_total,
                            3,
                            seed.wrapping_add(0x20),
                        ),
                        prefix_segments: split_total_across_segments(
                            prefix_total,
                            PREFIX_NAMESPACES.len(),
                            seed,
                        ),
                        suffix_segments: split_total_across_segments(
                            suffix_total,
                            SUFFIX_NAMESPACES.len(),
                            seed.wrapping_add(0x40),
                        ),
                        seed,
                    }
                })
            })
        })
        .boxed()
}

pub fn mid_row_case_strategy() -> BoxedStrategy<MidRowBatchCase> {
    prop_oneof![Just(4usize), Just(8usize), Just(32usize)]
        .prop_flat_map(|ods_width| {
            let max_batch_rows_full = ods_width - 2;
            (0..=max_batch_rows_full).prop_flat_map(move |batch_rows_full| {
                let max_prefix_rows = ods_width - 2 - batch_rows_full;
                (0..=max_prefix_rows, 1..ods_width, 1..ods_width, any::<u8>()).prop_map(
                    move |(prefix_rows, prefix_len, last_batch_len, seed)| MidRowBatchCase {
                        ods_width,
                        prefix_rows,
                        prefix_len,
                        batch_rows_full,
                        last_batch_len,
                        suffix_rows: ods_width - 2 - prefix_rows - batch_rows_full,
                        seed,
                    },
                )
            })
        })
        .boxed()
}

pub fn single_row_mid_row_case_strategy() -> BoxedStrategy<SingleRowMidRowCase> {
    prop_oneof![Just(4usize), Just(8usize)]
        .prop_flat_map(|ods_width| {
            (0..ods_width, 1..(ods_width - 2), any::<u8>()).prop_flat_map(
                move |(prefix_rows, prefix_len, seed)| {
                    let suffix_rows = ods_width - prefix_rows - 1;
                    (2..(ods_width - prefix_len)).prop_map(move |batch_len| SingleRowMidRowCase {
                        ods_width,
                        prefix_rows,
                        prefix_len,
                        batch_len,
                        suffix_rows,
                        seed,
                    })
                },
            )
        })
        .boxed()
}

pub fn single_row_absence_case_strategy() -> BoxedStrategy<SingleRowAbsenceCase> {
    prop_oneof![Just(4usize), Just(8usize), Just(32),]
        .prop_flat_map(|ods_width| {
            (1..ods_width, any::<u8>()).prop_map(move |(candidate_low_len, seed)| {
                SingleRowAbsenceCase {
                    ods_width,
                    candidate_low_len,
                    seed,
                }
            })
        })
        .boxed()
}

pub fn zero_row_absence_case_strategy() -> BoxedStrategy<ZeroRowAbsenceCase> {
    prop_oneof![Just(4usize), Just(8usize)]
        .prop_flat_map(|ods_width| {
            any::<u8>().prop_map(move |seed| ZeroRowAbsenceCase { ods_width, seed })
        })
        .boxed()
}

fn split_total_across_segments(total: usize, max_parts: usize, seed: u8) -> Vec<usize> {
    if total == 0 {
        return Vec::new();
    }

    let max_parts = max_parts.min(total);
    let parts = 1 + (seed as usize % max_parts);
    let mut segments = vec![1; parts];

    for offset in 0..(total - parts) {
        let idx = (seed as usize + offset) % parts;
        segments[idx] += 1;
    }

    segments
}

fn assert_canonical_ods_namespaces(ods_shares: &[Vec<u8>]) {
    for (idx, pair) in ods_shares.windows(2).enumerate() {
        assert!(
            pair[0].len() >= NS_SIZE && pair[1].len() >= NS_SIZE,
            "generated share is shorter than namespace prefix at index {idx}"
        );
        assert!(
            pair[0][..NS_SIZE] <= pair[1][..NS_SIZE],
            "generated ODS namespaces are not globally non-decreasing between shares {idx} and {}",
            idx + 1
        );
    }
}

fn build_canonical_block_from_ods(ods_shares: Vec<Vec<u8>>) -> FilteredCelestiaBlock {
    assert_canonical_ods_namespaces(&ods_shares);
    build_block_from_ods(ods_shares)
}
