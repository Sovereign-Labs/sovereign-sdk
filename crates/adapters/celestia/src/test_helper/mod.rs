pub(crate) mod docker;
pub(crate) mod files;
pub(crate) mod serialization;

use celestia_types::nmt::Namespace;

use crate::types::BlobWithSender;
use crate::verifier::address::CelestiaAddress;
use crate::verifier::RollupParams;

pub const ROLLUP_BATCH_NAMESPACE: Namespace = Namespace::const_v0(*b"\0\0sov-test");
pub const ROLLUP_PROOF_NAMESPACE: Namespace = Namespace::const_v0(*b"sov-test-p");

// Used for synthetically produced blocks
pub const ROLLUP_PARAMS_DEV: RollupParams = RollupParams {
    rollup_batch_namespace: ROLLUP_BATCH_NAMESPACE,
    rollup_proof_namespace: ROLLUP_PROOF_NAMESPACE,
};

const ROLLUP_OTHER_NAMESPACE_PRECEDING: Namespace = Namespace::const_v0(*b"\0\0sov-aaaa");
const ROLLUP_OTHER_NAMESPACE_FOLLOWING: Namespace = Namespace::const_v0(*b"\0\0sov-zzzz");

// Move those to keys/credentials.rs
pub const ADDR_1: &str = "celestia1a68m2l85zn5xh0l07clk4rfvnezhywc53g8x7s";
pub const ADDR_2: &str = "celestia1hvp2nfz3r6nqt8mlrzqf9ctwle942tkr0wql75";
pub const ADDR_3: &str = "celestia1w7wcupk5gswj25c0khnkey5fwmlndx6t5aarmk";
// One of the "mocha" addresses
pub const ADDR_4: &str = "celestia1vfpr5g7gfxawjdy0snrjku058vc8amumr29str";

pub(crate) fn blob_from_data(
    namespace: Namespace,
    data: Vec<u8>,
    signer: &CelestiaAddress,
) -> anyhow::Result<celestia_types::Blob> {
    celestia_types::blob::Blob::new(namespace, data, Some(signer.0)).map_err(Into::into)
}

/// Assert two blobs carry the same authenticated projection: the verified DA prefix, the
/// total DA length, and the identifying metadata.
///
/// [`BlobWithSender`] intentionally does NOT implement `PartialEq` — after witness pruning a
/// deserialized blob drops its shares, so structural equality is meaningless. Comparing the
/// projection here keeps that intent explicit. The derived `envelope_state` cache is excluded
/// (it is recomputed from the prefix, not identity-defining).
pub(crate) fn compare_equality(actual: &BlobWithSender, expected: &BlobWithSender) {
    assert_eq!(
        actual.compressed_verified_data(),
        expected.compressed_verified_data(),
        "blob verified DA prefix differs"
    );
    assert_eq!(
        actual.compressed_total_len(),
        expected.compressed_total_len(),
        "blob total DA length differs"
    );
    assert_eq!(
        actual.range_in_namespace, expected.range_in_namespace,
        "blob namespace range differs"
    );
    assert_eq!(actual.sender, expected.sender, "blob sender differs");
    assert_eq!(actual.hash, expected.hash, "blob hash differs");
}

/// Element-wise [`compare_equality`] over two blob lists.
pub(crate) fn compare_blobs(actual: &[BlobWithSender], expected: &[BlobWithSender]) {
    assert_eq!(actual.len(), expected.len(), "blob list length differs");
    for (a, e) in actual.iter().zip(expected) {
        compare_equality(a, e);
    }
}
