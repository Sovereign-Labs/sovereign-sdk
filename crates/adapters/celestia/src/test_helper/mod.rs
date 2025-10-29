pub(crate) mod docker;
pub(crate) mod files;
pub(crate) mod serialization;

use celestia_types::nmt::Namespace;

use crate::types::APP_VERSION;
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
    celestia_types::blob::Blob::new(namespace, data, Some(signer.0.clone()), APP_VERSION)
        .map_err(Into::into)
}
