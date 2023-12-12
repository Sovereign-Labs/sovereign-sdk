/// The namespace used by the rollup to store its data. This is a raw slice of 8 bytes.
/// The rollup stores its data in the namespace b"sov-test" on Celestia. Which in this case is encoded using the
/// ascii representation of each character.
pub const ROLLUP_BATCH_NAMESPACE_RAW: [u8; 10] = [0, 0, 115, 111, 118, 45, 116, 101, 115, 116];

///
pub const ROLLUP_PROOF_NAMESPACE_RAW: [u8; 10] = [0, 0, 22, 22, 22, 22, 22, 22, 22, 22];
