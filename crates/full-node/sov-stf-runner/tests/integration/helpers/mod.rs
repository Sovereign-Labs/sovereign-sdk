pub mod hash_stf;
pub mod runner_init;

/// Bytes of the genesis state root.
#[derive(Clone, Debug)]
pub struct RawGenesisStateRoot(pub Vec<u8>);

const GENESIS_STATE_ROOT: [u8; 32] = [22; 32];

pub fn genesis_state_root() -> RawGenesisStateRoot {
    RawGenesisStateRoot(GENESIS_STATE_ROOT.to_vec())
}
