/// The namespace used by the rollup to store its data. This is a raw slice of 8 bytes.
/// The rollup stores its data in the namespace b"sov-test" on Celestia. Which in this case is encoded using the
/// ascii representation of each character.
pub const ROLLUP_NAMESPACE_RAW: [u8; 10] = [0, 0, 115, 111, 118, 45, 116, 101, 115, 116];

/// The DA address of the sequencer (for now we use a centralized sequencer) in the tests.
/// Here this is the address of the sequencer on the celestia blockchain.
pub const SEQUENCER_DA_ADDRESS: &str = "celestia1a68m2l85zn5xh0l07clk4rfvnezhywc53g8x7s";
