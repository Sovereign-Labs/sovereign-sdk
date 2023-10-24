//! State manager for Prover and Zk Storage

use crate::prover_storage::ProverStateUpdate;
use crate::{config, MerkleProofSpec, ProverStorage, ZkStorage};

/// State manager for Prover and Zk Storage
pub struct SovStateManager<S: MerkleProofSpec> {
    state_db: sov_db::state_db::StateDB,
    native_db: sov_db::native_db::NativeDB,
    phantom_s: std::marker::PhantomData<S>,
}

impl<S: MerkleProofSpec> SovStateManager<S> {
    /// Create new SovStateManager from state config
    pub fn new(config: config::Config) -> anyhow::Result<Self> {
        let path = config.path;
        let state_db = sov_db::state_db::StateDB::with_path(&path)?;
        let native_db = sov_db::native_db::NativeDB::with_path(&path)?;
        Ok(Self {
            state_db,
            native_db,
            phantom_s: Default::default(),
        })
    }
}

#[cfg(feature = "native")]
impl<S: MerkleProofSpec> sov_rollup_interface::state::StateManager for SovStateManager<S> {
    type NativeState = ProverStorage<S>;
    type NativeChangeSet = ProverStateUpdate;
    type ZkState = ZkStorage<S>;

    fn get_native_state(&self) -> Self::NativeState {
        ProverStorage::with_db_handles(self.state_db.clone(), self.native_db.clone())
    }

    fn get_zk_state(&self) -> Self::ZkState {
        ZkStorage::new()
    }
}
