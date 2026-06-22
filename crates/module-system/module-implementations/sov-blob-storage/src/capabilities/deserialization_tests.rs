//! Tests for the native blob deserialization path: the lazy [`data_for_deserialization`] reader
//! (`LazyBlobReader`) and the [`is_borsh_truncated_input_error`] classifier that tells a
//! withheld/truncated blob apart from a genuinely malformed one.

use std::io::{Cursor, ErrorKind};
use std::num::NonZeroU8;
use std::sync::Arc;

use borsh::{BorshDeserialize, BorshSerialize};
use sov_mock_da::{MockAddress, MockHash};
use sov_mock_da::{MockBlob, MOCK_SEQUENCER_DA_ADDRESS};
use sov_modules_api::{BlobReaderTrait, FullyBakedTx};

use super::{
    blob_deserialization_gate, data_for_deserialization, is_borsh_truncated_input_error,
    BlobDeserGate, PreferredBatchData, PreferredProofData, BORSH_UNEXPECTED_LENGTH_OF_INPUT,
};

fn mock_blob(data: Vec<u8>) -> MockBlob {
    MockBlob::new_with_hash(data, MOCK_SEQUENCER_DA_ADDRESS.into())
}

/// Deserializes `B` through the production native reader (`LazyBlobReader`), leaving `blob`
/// borrowable afterwards so the test can inspect how many bytes were verified.
fn read_via_lazy<B: BorshDeserialize>(blob: &mut impl BlobReaderTrait) -> std::io::Result<B> {
    let mut reader = data_for_deserialization(blob);
    B::try_from_reader(&mut reader)
}

#[derive(Debug, borsh::BorshDeserialize)]
#[allow(dead_code)] // only ever used as a (failing) deserialization target
enum TwoVariants {
    First,
    Second,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct RecordingBlob {
    data: Vec<u8>,
    verified_len: usize,
    max_advance_request: usize,
}

impl RecordingBlob {
    fn new(data: Vec<u8>) -> Self {
        Self {
            data,
            verified_len: 0,
            max_advance_request: 0,
        }
    }
}

impl BlobReaderTrait for RecordingBlob {
    type Address = MockAddress;
    type BlobHash = MockHash;

    fn sender(&self) -> Self::Address {
        MockAddress::new(MOCK_SEQUENCER_DA_ADDRESS)
    }

    fn hash(&self) -> Self::BlobHash {
        MockHash([0; 32])
    }

    fn verified_data(&self) -> &[u8] {
        &self.data[..self.verified_len]
    }

    fn total_len(&self) -> usize {
        self.data.len()
    }

    fn advance(&mut self, num_bytes: usize) -> &[u8] {
        self.max_advance_request = self.max_advance_request.max(num_bytes);
        self.verified_len = self
            .verified_len
            .saturating_add(num_bytes)
            .min(self.data.len());
        self.verified_data()
    }
}

// `is_borsh_truncated_input_error`: telling "ran out of input" apart from other failures.

#[test]
fn truncated_vec_u8_is_classified_as_truncation() {
    // Borsh Vec<u8>: first 4 bytes are little-endian length. This claims length 4 but provides
    // only one payload byte, which borsh reports as InvalidData("Unexpected length of input").
    let error = Vec::<u8>::try_from_reader(&mut Cursor::new([4, 0, 0, 0, 0xaa])).unwrap_err();

    assert_eq!(error.kind(), ErrorKind::InvalidData);
    assert_eq!(error.to_string(), "Unexpected length of input");
    assert!(is_borsh_truncated_input_error(&error));
}

#[test]
fn huge_vec_u8_claim_does_not_drive_huge_lazy_read() {
    const BORSH_VEC_U8_INITIAL_READ_CAP: usize = 1024 * 1024;
    let claimed_len = BORSH_VEC_U8_INITIAL_READ_CAP as u32 + 1;
    let mut blob = RecordingBlob::new(claimed_len.to_le_bytes().to_vec());

    let error = read_via_lazy::<Vec<u8>>(&mut blob).unwrap_err();

    assert_eq!(error.kind(), ErrorKind::InvalidData);
    assert_eq!(error.to_string(), "Unexpected length of input");
    assert!(
        blob.max_advance_request <= BORSH_VEC_U8_INITIAL_READ_CAP,
        "borsh asked the lazy reader for {} bytes from a {} byte claim",
        blob.max_advance_request,
        claimed_len
    );
    assert!(
        blob.max_advance_request < claimed_len as usize,
        "borsh must not eagerly read the full claimed length"
    );
}

#[test]
fn raw_unexpected_eof_is_classified_as_truncation() {
    // Some deserializers propagate the reader's raw UnexpectedEof instead of borsh's
    // "Unexpected length of input"; the classifier must catch that shape too.
    let error = std::io::Error::new(ErrorKind::UnexpectedEof, "failed to fill whole buffer");

    assert!(is_borsh_truncated_input_error(&error));
}

#[test]
fn other_invalid_data_is_not_truncation() {
    // 0x02 is not a valid bool: malformation, not truncation.
    let error = bool::try_from_reader(&mut Cursor::new([2])).unwrap_err();

    assert_eq!(error.kind(), ErrorKind::InvalidData);
    assert!(!is_borsh_truncated_input_error(&error));
}

#[test]
fn trailing_garbage_is_not_truncation() {
    // A valid value followed by an extra byte decodes fine but leaves bytes unread; that is
    // malformation, not truncation.
    let mut serialized = borsh::to_vec(&FullyBakedTx::new(vec![1, 2, 3])).unwrap();
    serialized.push(0xff);
    let mut blob = mock_blob(serialized);

    let error = read_via_lazy::<FullyBakedTx>(&mut blob).unwrap_err();

    assert_eq!(error.kind(), ErrorKind::InvalidData);
    assert_eq!(error.to_string(), "Not all bytes read");
    assert!(!is_borsh_truncated_input_error(&error));
}

// `LazyBlobReader`: how much of the blob gets verified on success and on failure.

#[test]
fn valid_blob_deserializes_and_consumes_entire_blob() {
    // Multi-element vector with multi-byte, asymmetric values exercises repeated reads/advances.
    let value: Vec<u32> = vec![0x11223344, 0x55667788, 0x99aabbcc];
    let mut blob = mock_blob(borsh::to_vec(&value).unwrap());

    let decoded: Vec<u32> = read_via_lazy(&mut blob).expect("valid blob should deserialize");

    assert_eq!(decoded, value);
    assert_eq!(
        blob.verified_data().len(),
        blob.total_len(),
        "a successful deserialization must verify the entire blob (borsh requires EOF)"
    );
}

#[test]
fn truncated_blob_is_fully_verified_before_reporting_truncation() {
    // The production guard in `deserialize_or_try_slash_sender` only trusts a truncation error if
    // the whole blob was verified. Cover both shapes borsh uses for "ran out of input" — a Vec<u8>
    // whose length prefix over-promises (InvalidData "Unexpected length of input") and a struct
    // whose trailing fixed-size field is cut off (raw UnexpectedEof) — across the payload types the
    // production path actually deserializes.
    //
    // Keep this list in sync with the `impl BlobPayload` block in `capabilities.rs`: every
    // implementor is allowed past the truncation guard, so every implementor must be proven here.
    assert_truncated_blob_is_fully_verified(vec![1u8, 2, 3]);
    assert_truncated_blob_is_fully_verified(PreferredBatchData {
        sequence_number: 0x1234,
        data: Arc::new(vec![FullyBakedTx::new(vec![1, 2, 3])]),
        visible_slots_to_advance: NonZeroU8::new(1).unwrap(),
    });
    assert_truncated_blob_is_fully_verified(FullyBakedTx::new(vec![1, 2, 3]));
    assert_truncated_blob_is_fully_verified(PreferredProofData {
        sequence_number: 1,
        data: vec![1, 2, 3],
    });
    assert_truncated_blob_is_fully_verified(vec![FullyBakedTx::new(vec![1, 2, 3])]);
}

fn assert_truncated_blob_is_fully_verified<T: BorshSerialize + BorshDeserialize>(value: T) {
    let mut serialized = borsh::to_vec(&value).unwrap();
    serialized
        .pop()
        .expect("test values must serialize to non-empty bytes");
    let mut blob = mock_blob(serialized);

    // `T` need not be `Debug`, so match rather than `unwrap_err`.
    let error = match read_via_lazy::<T>(&mut blob) {
        Ok(_) => panic!("truncated bytes unexpectedly deserialized"),
        Err(error) => error,
    };

    assert!(
        is_borsh_truncated_input_error(&error),
        "truncated blob was not classified as truncated input: {error:?}"
    );
    assert_eq!(
        blob.verified_data().len(),
        blob.total_len(),
        "borsh must consume the whole blob before reporting truncated input"
    );
}

#[test]
fn malformed_blob_fails_early_without_verifying_the_rest() {
    // First byte 0x07 is not a valid variant tag (only 0 and 1 exist); the remaining 500 bytes are
    // padding an honest reader must never need to verify.
    let mut bytes = vec![0u8; 501];
    bytes[0] = 0x07;
    let mut blob = mock_blob(bytes);

    let error = read_via_lazy::<TwoVariants>(&mut blob).unwrap_err();

    assert_eq!(error.kind(), ErrorKind::InvalidData);
    assert!(!is_borsh_truncated_input_error(&error));
    assert!(
        blob.verified_data().len() < blob.total_len(),
        "reader verified {} of {} bytes; it must bail out early on malformed input",
        blob.verified_data().len(),
        blob.total_len(),
    );
}

#[test]
fn classifier_false_positive_before_reader_is_exhausted() {
    // The classifier keys off the error message, so a deserializer that emits borsh's
    // "Unexpected length of input" before draining the reader trips it while bytes remain
    // unverified. This is why the production guard trusts a truncation verdict only when the whole
    // blob was verified, not on the classifier alone.
    #[derive(Debug)]
    struct EarlyUnexpectedLengthError;

    impl BorshDeserialize for EarlyUnexpectedLengthError {
        fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
            let mut first_byte = [0u8; 1];
            reader.read_exact(&mut first_byte)?;
            Err(std::io::Error::new(
                ErrorKind::InvalidData,
                BORSH_UNEXPECTED_LENGTH_OF_INPUT,
            ))
        }
    }

    let mut blob = mock_blob(vec![1, 2, 3]);
    let error = read_via_lazy::<EarlyUnexpectedLengthError>(&mut blob).unwrap_err();

    assert!(is_borsh_truncated_input_error(&error));
    assert!(
        blob.verified_data().len() < blob.total_len(),
        "a classifier false positive can happen before the native reader consumes the full blob"
    );
}

#[test]
fn empty_blob_is_truncation_without_panicking() {
    // Boundary: an empty blob exhausts immediately. The reader must not panic or loop, and
    // verified == total == 0 so the production guard would not fire.
    let mut blob = mock_blob(Vec::new());

    let error = read_via_lazy::<u32>(&mut blob).unwrap_err();

    assert!(is_borsh_truncated_input_error(&error));
    assert_eq!(blob.verified_data().len(), blob.total_len());
}

#[test]
fn multibyte_trailing_garbage_proceeds_to_the_slash_path() {
    // A registered sequencer posts a valid value followed by *two* trailing bytes. The lazy
    // native reader decodes the value and probes a single byte, so it stops with
    // `verified_data().len() < total_len()` and a non-truncation `Err`. With the deserialize
    // outcome an `Err`, the gate must return `Proceed` (the error arm then slashes the sender) —
    // not `WithheldFailClosed`, which would panic-halt the node on a fully-present, sender-
    // malformed blob. The single-trailing-byte case hides this: the 1-byte probe makes
    // `verified == total`, so the gap only shows with two or more trailing bytes.
    let mut serialized = borsh::to_vec(&FullyBakedTx::new(vec![1, 2, 3])).unwrap();
    serialized.push(0xaa);
    serialized.push(0xbb);
    let mut blob = mock_blob(serialized);

    let result = read_via_lazy::<FullyBakedTx>(&mut blob);
    let error = result
        .as_ref()
        .expect_err("a valid value followed by trailing bytes must fail to deserialize");
    assert_eq!(error.to_string(), "Not all bytes read");
    assert!(!is_borsh_truncated_input_error(error));
    assert!(
        blob.verified_data().len() < blob.total_len(),
        "lazy reader should stop after the 1-byte probe: {} of {} verified",
        blob.verified_data().len(),
        blob.total_len(),
    );

    assert_eq!(
        blob_deserialization_gate(
            blob.rollup_decode_failed(),
            result.is_ok(),
            blob.verified_data().len(),
            blob.total_len(),
        ),
        BlobDeserGate::Proceed,
        "fully-present multi-byte trailing garbage must reach the slash path, not fail closed",
    );
}
