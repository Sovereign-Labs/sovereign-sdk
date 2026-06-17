//! Tests for the native blob deserialization path: the lazy [`data_for_deserialization`] reader
//! (`LazyBlobReader`) and the [`is_borsh_truncated_input_error`] classifier that tells a
//! withheld/truncated blob apart from a genuinely malformed one.

use std::io::{Cursor, ErrorKind};
use std::num::NonZeroU8;
use std::sync::Arc;

use borsh::{BorshDeserialize, BorshSerialize};
use sov_mock_da::{MockBlob, MOCK_SEQUENCER_DA_ADDRESS};
use sov_modules_api::{BlobReaderTrait, FullyBakedTx};

use super::{data_for_deserialization, is_borsh_truncated_input_error, PreferredBatchData};

fn mock_blob(data: Vec<u8>) -> MockBlob {
    MockBlob::new_with_hash(data, MOCK_SEQUENCER_DA_ADDRESS.into())
}

/// Deserializes `B` through the production native reader (`LazyBlobReader`), leaving `blob`
/// borrowable afterwards so the test can inspect how many bytes were verified.
fn read_via_lazy<B: BorshDeserialize>(blob: &mut MockBlob) -> std::io::Result<B> {
    let mut reader = data_for_deserialization(blob);
    B::try_from_reader(&mut reader)
}

#[derive(Debug, borsh::BorshDeserialize)]
#[allow(dead_code)] // only ever used as a (failing) deserialization target
enum TwoVariants {
    First,
    Second,
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
    // the whole blob was verified. Cover both shapes borsh uses for "ran out of input": a Vec<u8>
    // whose length prefix over-promises (InvalidData "Unexpected length of input") and a struct
    // whose trailing fixed-size field is cut off (raw UnexpectedEof).
    assert_truncated_blob_is_fully_verified(vec![1u8, 2, 3]);
    assert_truncated_blob_is_fully_verified(PreferredBatchData {
        sequence_number: 0x1234,
        data: Arc::new(vec![FullyBakedTx::new(vec![1, 2, 3])]),
        visible_slots_to_advance: NonZeroU8::new(1).unwrap(),
    });
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
fn empty_blob_is_truncation_without_panicking() {
    // Boundary: an empty blob exhausts immediately. The reader must not panic or loop, and
    // verified == total == 0 so the production guard would not fire.
    let mut blob = mock_blob(Vec::new());

    let error = read_via_lazy::<u32>(&mut blob).unwrap_err();

    assert!(is_borsh_truncated_input_error(&error));
    assert_eq!(blob.verified_data().len(), blob.total_len());
}
