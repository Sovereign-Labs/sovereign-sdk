use crate::Sequencer;
use anyhow::Result;
use sov_modules_api::ModuleInfo;
use sov_state::WorkingSet;

pub struct Hooks<C: sov_modules_api::Context> {
    inner: Sequencer<C>,
}

impl<C: sov_modules_api::Context> Hooks<C> {
    pub fn new() -> Self {
        Self {
            inner: Sequencer::new(),
        }
    }

    pub fn lock(&self, working_set: &mut WorkingSet<C::Storage>) -> Result<()> {
        // TODO:
        // Transfer `self.inner.coins_to_lock` form `self.inner.seq_rollup_address` to `self.address`
        todo!()
    }

    pub fn next_sequencer(&self, working_set: &mut WorkingSet<C::Storage>) -> Result<Vec<u8>> {
        Ok(self.inner.da_address.get_or_err(working_set)?)
    }

    pub fn slash(&self, working_set: &mut WorkingSet<C::Storage>) -> Result<()> {
        todo!()
    }

    pub fn reward(&self, amount: u64, working_set: &mut WorkingSet<C::Storage>) -> Result<()> {
        // TODO:
        // Transfer `self.coins_to_lock` form  `self.inner.address` to `self.seq_rollup_address`
        // Add `amount` coins to self.inner.seq_rollup_address
        todo!()
    }
}
