//! Tests for the lazy, partial-verification blob deserialization path: the native
//! `LazyBlobReader` (via [`data_for_deserialization`]) and the truncation guard that
//! distinguishes a malformed blob from a withheld/truncated one.
//!
//! These complement the classifier-level tests in the sibling `tests` module: here we drive
//! borsh over a real [`MockBlob`] and assert *how much* of the blob gets verified, which is the
//! whole point of the change (a blob that fails early must not force verification of the rest).

use std::io::ErrorKind;

use borsh::BorshDeserialize;
use sov_mock_da::{MockBlob, MOCK_SEQUENCER_DA_ADDRESS};
use sov_modules_api::BlobReaderTrait;

use super::{data_for_deserialization, is_borsh_truncated_input_error};

fn mock_blob(data: Vec<u8>) -> MockBlob {
    MockBlob::new_with_hash(data, MOCK_SEQUENCER_DA_ADDRESS.into())
}

/// Deserializes `B` through the production native reader (`LazyBlobReader`), leaving `blob`
/// borrowable afterwards so the test can inspect how many bytes were verified.
fn read_via_lazy<B: BorshDeserialize>(blob: &mut MockBlob) -> std::io::Result<B> {
    let mut reader = data_for_deserialization(blob);
    B::try_from_reader(&mut reader)
}

#[derive(Debug, PartialEq, borsh::BorshSerialize, borsh::BorshDeserialize)]
struct Sample {
    a: u32,
    b: u16,
}

#[derive(Debug, borsh::BorshDeserialize)]
#[allow(dead_code)] // only ever used as a (failing) deserialization target
enum TwoVariants {
    First,
    Second,
}

#[test]
fn valid_blob_deserializes_and_consumes_entire_blob() {
    let value = Sample {
        a: 0x12345678,
        b: 0xabcd,
    };
    let mut blob = mock_blob(borsh::to_vec(&value).unwrap());

    let decoded: Sample = read_via_lazy(&mut blob).expect("valid blob should deserialize");

    assert_eq!(decoded, value);
    assert_eq!(
        blob.verified_data().len(),
        blob.total_len(),
        "a successful deserialization must verify the entire blob (borsh requires EOF)"
    );
}

#[test]
fn valid_vec_blob_round_trips_through_lazy_reader() {
    // Multi-element vector with multi-byte, asymmetric values exercises repeated reads/advances.
    let value: Vec<u32> = vec![0x11223344, 0x55667788, 0x99aabbcc];
    let mut blob = mock_blob(borsh::to_vec(&value).unwrap());

    let decoded: Vec<u32> = read_via_lazy(&mut blob).expect("valid blob should deserialize");

    assert_eq!(decoded, value);
    assert_eq!(blob.verified_data().len(), blob.total_len());
}

#[test]
fn malformed_blob_fails_early_and_verifies_only_the_consumed_prefix() {
    // First byte 0x07 is not a valid variant tag (only 0 and 1 exist); the remaining 500 bytes
    // are padding an honest reader must never need to verify.
    let mut bytes = vec![0u8; 501];
    bytes[0] = 0x07;
    let mut blob = mock_blob(bytes);

    let error = read_via_lazy::<TwoVariants>(&mut blob).unwrap_err();

    assert_eq!(error.kind(), ErrorKind::InvalidData);
    assert!(
        !is_borsh_truncated_input_error(&error),
        "an invalid variant tag is genuine malformation, not truncated input: {error:?}"
    );
    assert!(
        blob.verified_data().len() < blob.total_len(),
        "deserialization must bail out without verifying the whole blob (verified {} of {})",
        blob.verified_data().len(),
        blob.total_len(),
    );
}

#[test]
fn empty_blob_through_lazy_reader_is_truncation_without_panicking() {
    // Boundary: an empty blob exhausts immediately. The reader must not panic or loop, the error
    // must classify as truncation, and (verified == total == 0) so the guard would not fire.
    let mut blob = mock_blob(Vec::new());

    let error = read_via_lazy::<u32>(&mut blob).unwrap_err();

    assert!(is_borsh_truncated_input_error(&error));
    assert_eq!(blob.verified_data().len(), blob.total_len());
}

#[test]
fn classifier_recognizes_raw_unexpected_eof() {
    // borsh maps most short reads to InvalidData("Unexpected length of input"), but some
    // deserializers propagate the reader's raw UnexpectedEof; the classifier must catch that too.
    let error = std::io::Error::new(ErrorKind::UnexpectedEof, "failed to fill whole buffer");

    assert!(is_borsh_truncated_input_error(&error));
}
