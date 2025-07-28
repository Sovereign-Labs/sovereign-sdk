use sov_modules_api::macros::config_value;
use sov_modules_api::{BatchWithId, DaSpec, Spec};
use tracing::{error, warn};

use crate::{SequencerType, ValidatedBlob};

/// We put a cap on how much space preferred blobs can use, but we allow
/// non-preferred blobs to use any and all space if needed.
struct BlobSizeChecker {
    remaining_preferred_size: u32,
    remaining_non_preferred_size: u32,
}

impl BlobSizeChecker {
    fn can_process_blob(&self, sequencer_type: SequencerType, blob_len: usize) -> bool {
        // On certain zkVM platforms, usize is equivalent to u32. While we check for overflows here,
        // it is practically infeasible to encounter batches exceeding 2^32 bytes.
        let Ok(blob_len) = u32::try_from(blob_len) else {
            error!(
                blob_len = %blob_len,
               "BlobSizeChecker: Blob length is, bigger than u32::MAX, this should never happen. The blob was dropped.",
            );

            return false;
        };

        let counter = match sequencer_type {
            SequencerType::Preferred => &self.remaining_preferred_size,
            SequencerType::NonPreferred => &self.remaining_non_preferred_size,
        };

        let ok = counter.checked_sub(blob_len).is_some();

        if !ok {
            // The full-node/prover is capable of processing batches of up to hundreds of megabytes in size.
            // However, the slot limits on the DAs are in the range of single megabytes, making it impossible to reach these limits.
            // These checks are added as a precaution, but such a scenario should not occur.
            warn!(
                %blob_len,
                ?sequencer_type,
                %self.remaining_preferred_size,
                %self.remaining_non_preferred_size,
               "BlobSizeChecker: Unable to accumulate the blob due to size past limits.",
            );
        }

        ok
    }

    /// Processes a blob of the given size **after** the call to
    /// [`BlobSizeChecker::can_accept_blob`].
    ///
    /// # Panics
    ///
    /// Panics if the blob size overflows i.e. the necessary checks were not
    /// been performed.
    fn process_blob(&mut self, sequencer_type: SequencerType, blob_len: usize) {
        let blob_len = u32::try_from(blob_len).expect("Bad number cast");

        let counter = match sequencer_type {
            SequencerType::Preferred => &mut self.remaining_preferred_size,
            SequencerType::NonPreferred => &mut self.remaining_non_preferred_size,
        };

        // SAFETY: we checked that the blob size checker has capacity for
        // this blob in the call to `can_accept_blob`, which ensures that this
        // operation will not underflow.

        *counter = counter.checked_sub(blob_len).unwrap();
    }
}

/// This component limits the data returned from the `BlobSelector` to the `STF` blueprint.
/// While the `STF` blueprint can handle data sizes in the range of hundreds of megabytes, the DA layer block sizes are typically only a few megabytes.
/// As a result, this mechanism acts as a precautionary sanity check.
/// Example
/// If the maximum allowed size is 10MB, and the blobs are inserted in the following order [3MB, 20MB, 1MB], the `BlobsWithTotalSizeLimit` will only hold the first and the last blobs because their total size is less than 10MB.
pub(crate) struct BlobsAccumulatorWithSizeLimit<S: Spec> {
    blobs_with_address: Vec<ValidatedBlob<S, BatchWithId<S>>>,
    blob_size_checker: BlobSizeChecker,
}

#[derive(Debug, Eq, PartialEq, Clone)]
pub(crate) enum PushOrIgnore<S: Spec> {
    Accepted,
    IgnoredBlob([u8; 32], <<S as Spec>::Da as DaSpec>::Address),
}

impl<S: Spec> BlobsAccumulatorWithSizeLimit<S> {
    pub fn new() -> Self {
        Self::new_with_size(config_value!(
            "MAX_ALLOWED_DATA_SIZE_RETURNED_BY_BLOB_STORAGE"
        ))
    }

    fn new_with_size(max_total_size: u32) -> Self {
        let max_preferred_blob_size = max_total_size
            .checked_div(sov_modules_api::PREFERRED_DATA_FRACTION.denominator)
            .expect("Can't devide by 0")
            .checked_mul(sov_modules_api::PREFERRED_DATA_FRACTION.numerator)
            // This cannot overflow because the PREFERRED_DATA_FRACTION must be less than 1.
            .unwrap();

        assert!(max_preferred_blob_size <= max_total_size);

        Self {
            blobs_with_address: Vec::new(),
            blob_size_checker: BlobSizeChecker {
                remaining_preferred_size: max_preferred_blob_size,
                remaining_non_preferred_size: max_total_size
                    .saturating_sub(max_preferred_blob_size),
            },
        }
    }

    /// Stores the blob internally if the total size of stored blobs didn't
    /// cross the preconfigured limit.
    pub fn push_or_ignore(
        &mut self,
        sequencer_type: SequencerType,
        elem: ValidatedBlob<S, BatchWithId<S>>,
    ) -> PushOrIgnore<S> {
        let can_process_blob = self
            .blob_size_checker
            .can_process_blob(sequencer_type, elem.blob.blob_size());

        if can_process_blob {
            self.blob_size_checker
                .process_blob(sequencer_type, elem.blob.blob_size());
            self.blobs_with_address.push(elem);
            PushOrIgnore::Accepted
        } else {
            PushOrIgnore::IgnoredBlob(elem.blob.id(), elem.sender)
        }
    }

    /// Returns true if the blob can be accepted.
    pub(crate) fn can_accept_blob(&self, sequencer_type: SequencerType, blob_len: usize) -> bool {
        self.blob_size_checker
            .can_process_blob(sequencer_type, blob_len)
    }

    pub(crate) fn inner(self) -> Vec<ValidatedBlob<S, BatchWithId<S>>> {
        self.blobs_with_address
    }
}

#[cfg(test)]
mod tests {
    use sov_modules_api::{BatchWithId, BlobDataWithId, FullyBakedTx};
    use sov_test_utils::assert_matches;

    use super::*;
    use crate::{Escrow, SequencerType};

    pub type S = sov_test_utils::TestSpec;

    fn make_tx_batches_of_given_size(sizes: Vec<usize>) -> Vec<Vec<FullyBakedTx>> {
        let mut batches_of_txs = Vec::new();

        for size in sizes {
            let tx = FullyBakedTx::new(vec![0; size]);
            batches_of_txs.push(vec![tx]);
        }

        batches_of_txs
    }

    fn create_blob(size: usize) -> ValidatedBlob<S, BatchWithId<S>> {
        ValidatedBlob::new(
            BlobDataWithId::Batch(BatchWithId::new(
                vec![FullyBakedTx::new(vec![0; size])].into(),
                [0; 32],
                [0; 28].into(),
            )),
            [0; 32].into(),
            Escrow::None,
        )
    }

    #[test]
    fn test_blob_outputs() {
        fn test_helper_correct_blob_selection_outputs(
            blob_sizes: Vec<usize>,
            expected_indexes: Vec<u8>,
            max_size: u32,
        ) {
            let mut blobs_with_total_size_limit =
                BlobsAccumulatorWithSizeLimit::<S>::new_with_size(max_size);
            let txs = make_tx_batches_of_given_size(blob_sizes);

            let mut expected_addresses = Vec::new();
            for (i, b) in txs.into_iter().enumerate() {
                let addr = [i as u8; 32].into();
                let b = ValidatedBlob::new(
                    BlobDataWithId::Batch(BatchWithId::new(
                        b.into(),
                        [0; 32],
                        [i as u8; 28].into(),
                    )),
                    addr,
                    Escrow::None,
                );
                if expected_indexes.contains(&(i as u8)) {
                    expected_addresses.push(addr);
                }
                blobs_with_total_size_limit.push_or_ignore(SequencerType::Preferred, b);
            }

            let inner = blobs_with_total_size_limit
                .inner()
                .into_iter()
                .map(|b| b.sender)
                .collect::<Vec<_>>();

            assert_eq!(inner, expected_addresses);
        }

        test_helper_correct_blob_selection_outputs(vec![1, 2, 3], vec![0, 1, 2], 300);
        test_helper_correct_blob_selection_outputs(vec![1, 14, 11], vec![0], 80);
        test_helper_correct_blob_selection_outputs(vec![111, 2, 3], vec![1, 2], 140);
        test_helper_correct_blob_selection_outputs(vec![10, 2, 3], vec![0], 100);
        test_helper_correct_blob_selection_outputs(vec![3, 220, 1, 880, 70], vec![0, 2], 140);
        test_helper_correct_blob_selection_outputs(vec![10], vec![0], 100000);
        test_helper_correct_blob_selection_outputs(vec![0], vec![0], 80);
    }

    #[test]
    fn test_preferred_blob_outputs() {
        let max_size = 100;

        let mut blobs_with_total_size_limit =
            BlobsAccumulatorWithSizeLimit::<S>::new_with_size(max_size);

        let blob_size = 20;

        // The blob is too large to be processed as a non-preferred blob.
        {
            let blob = create_blob(blob_size);
            let can_process_blob = blobs_with_total_size_limit
                .push_or_ignore(SequencerType::NonPreferred, blob.clone());
            assert_matches!(can_process_blob, PushOrIgnore::IgnoredBlob(_, _));
        }

        // The blob can be processed as a preferred blob.
        {
            let blob = create_blob(blob_size);
            let can_process_blob =
                blobs_with_total_size_limit.push_or_ignore(SequencerType::Preferred, blob);
            assert_eq!(can_process_blob, PushOrIgnore::Accepted);
        }
    }

    #[test]
    fn test_preferred_and_standard_blob() {
        let max_size = 800;

        let mut blobs_with_total_size_limit =
            BlobsAccumulatorWithSizeLimit::<S>::new_with_size(max_size);

        let blob_size = 80;
        let blob = create_blob(blob_size);

        {
            let can_process_blob =
                blobs_with_total_size_limit.push_or_ignore(SequencerType::Preferred, blob);
            assert_eq!(can_process_blob, PushOrIgnore::Accepted);
        }

        let blob_size = 10;
        let blob = create_blob(blob_size);

        {
            let can_process_blob =
                blobs_with_total_size_limit.push_or_ignore(SequencerType::NonPreferred, blob);
            assert_eq!(can_process_blob, PushOrIgnore::Accepted);
        }
    }
}
