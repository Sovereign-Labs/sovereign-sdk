use crate::Sequencer;
use anyhow::Result;
use sov_modules_api::ModuleInfo;
use sov_state::WorkingSet;

/// Sequencer hooks description:
/// At the beginning of SDK's `apply_batch` we need to lock some amount of sequencer funds which will be returned
/// along with additional reward upon successful batch execution. If the sequencer is malicious the funds are slashed (remain locked forever).
pub struct Hooks<C: sov_modules_api::Context> {
    inner: Sequencer<C>,
}

impl<C: sov_modules_api::Context> Hooks<C> {
    pub fn new() -> Self {
        Self {
            inner: Sequencer::new(),
        }
    }

    /// Locks the sequencer coins.
    pub fn lock(&self, working_set: &mut WorkingSet<C::Storage>) -> Result<()> {
        let sequencer = &self.inner.seq_rollup_address.get_or_err(working_set)?;
        let locker = &self.inner.address;
        let coins = self.inner.coins_to_lock.get_or_err(working_set)?;

        self.inner
            .bank
            .transfer_from(sequencer, locker, coins, working_set)?;

        Ok(())
    }

    /// Currently this module supports only centralized sequencer, therefore this method always returns the same DA address.
    pub fn next_sequencer(&self, working_set: &mut WorkingSet<C::Storage>) -> Result<Vec<u8>> {
        Ok(self.inner.da_address.get_or_err(working_set)?)
    }

    /// Unlocks the sequencer coins and awards additional coins (possibly based on transactions fees and used gas).
    /// TODO: The `amount` field represents the additional award. As of now, we are not using it because we need to implement
    /// the gas and TX fees mechanism first. See: issue number
    pub fn reward(&self, _amount: u64, working_set: &mut WorkingSet<C::Storage>) -> Result<()> {
        let sequencer = &self.inner.seq_rollup_address.get_or_err(working_set)?;
        let locker = &self.inner.address;
        let coins = self.inner.coins_to_lock.get_or_err(working_set)?;

        self.inner
            .bank
            .transfer_from(locker, sequencer, coins, working_set)?;

        Ok(())
    }
}
