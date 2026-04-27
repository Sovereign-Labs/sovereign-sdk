use anyhow::Result;
use sov_modules_api::{GenesisState, Spec};

use super::HooksCount;

impl<S: Spec> HooksCount<S> {
    /// Initializes module with the `admin` role.
    pub(crate) fn init_module(&mut self, state: &mut impl GenesisState<S>) -> Result<()> {
        self.begin_rollup_block_hook_count.set(&0, state)?;
        self.end_rollup_block_hook_count.set(&0, state)?;
        self.finalize_hook_count.set(&0, state)?;
        Ok(())
    }
}
