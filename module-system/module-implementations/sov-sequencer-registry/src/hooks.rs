use crate::Sequencer;
use anyhow::Result;
use sov_modules_api::{
    hooks::{ApplyBatchHooks, Transaction},
    Context, ModuleInfo, Spec,
};
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
        Ok(self.inner.seq_da_address.get_or_err(working_set)?)
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

impl<C: Context> ApplyBatchHooks for Sequencer<C> {
    type Context = C;

    fn pre_dispatch_tx_hook(
        &self,
        tx: Transaction<C>,
        working_set: &mut WorkingSet<<Self::Context as Spec>::Storage>,
    ) -> anyhow::Result<<Self::Context as Spec>::Address> {
        todo!()
    }

    fn post_dispatch_tx_hook(
        &self,
        pub_key: <Self::Context as Spec>::PublicKey,
        working_set: &mut WorkingSet<<Self::Context as sov_modules_api::Spec>::Storage>,
    ) {
        todo!()
    }

    fn enter_apply_blob(
        &self,
        sequencer_da: &[u8],
        working_set: &mut WorkingSet<<Self::Context as sov_modules_api::Spec>::Storage>,
    ) -> anyhow::Result<()> {
        let next_sequencer_da = self.seq_da_address.get_or_err(working_set);

        match next_sequencer_da {
            Ok(next_sequencer_da) => {
                if next_sequencer_da != sequencer_da {
                    anyhow::bail!("Invalid next sequencer.")
                }
            }
            Err(_) => anyhow::bail!("Sequencer {:?} not registered. ", sequencer_da),
        }

        let sequencer = &self.seq_rollup_address.get_or_err(working_set)?;
        let locker = &self.address;
        let coins = self.coins_to_lock.get_or_err(working_set)?;

        self.bank
            .transfer_from(sequencer, locker, coins, working_set)?;

        Ok(())
    }

    fn exit_apply_blob(
        &self,
        amount: u64,
        working_set: &mut WorkingSet<<Self::Context as sov_modules_api::Spec>::Storage>,
    ) -> anyhow::Result<()> {
        // self.sequencer_hooks.reward(amount, working_set)
        todo!()
    }
}
