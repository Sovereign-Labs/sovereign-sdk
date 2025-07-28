use anyhow::Result;
use sov_modules_api::{GenesisState, Spec};

use super::{SyntheticLoad, VERY_LARGE_VEC_LENGTH};

impl<S: Spec> SyntheticLoad<S> {
    /// Initializes module with the `admin` role.
    pub(crate) fn init_module(&mut self, state: &mut impl GenesisState<S>) -> Result<()> {
        let genesis_vec = (0..VERY_LARGE_VEC_LENGTH).collect::<Vec<_>>();
        self.very_large_vec.set_all(genesis_vec, state)?;
        Ok(())
    }
}
