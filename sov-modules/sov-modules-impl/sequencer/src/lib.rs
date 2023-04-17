mod genesis;
pub mod hooks;
use sov_modules_api::Error;
use sov_modules_macros::ModuleInfo;
use sov_state::{StateValue, WorkingSet};

/// Initial configuration for Sequencer module.
pub struct SequencerConfig<C: sov_modules_api::Context> {
    pub seq_rollup_address: C::Address,
    pub seq_da_address: Vec<u8>,
    pub coins_to_lock: bank::Coins<C::Address>,
}

#[derive(ModuleInfo)]
pub struct Sequencer<C: sov_modules_api::Context> {
    #[address]
    pub(crate) address: C::Address,

    #[module]
    pub(crate) bank: bank::Bank<C>,

    /// The sequencer address on the rollup.
    #[state]
    pub(crate) seq_rollup_address: StateValue<C::Address>,

    /// The sequencer address on the DA.
    #[state]
    pub(crate) da_address: StateValue<Vec<u8>>,

    /// Coin's that will be slashed if the sequencer is malicious.
    /// The coins will be transferred from `self.seq_rollup_address` to `self.address`
    /// and locked forever.
    #[state]
    pub(crate) coins_to_lock: StateValue<bank::Coins<C::Address>>,
}

impl<C: sov_modules_api::Context> sov_modules_api::Module for Sequencer<C> {
    type Context = C;

    type Config = SequencerConfig<C>;

    fn genesis(
        &self,
        config: &Self::Config,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<(), Error> {
        Ok(self.init_module(config, working_set)?)
    }

    // Questions:
    // 1. There is no need to handle external calls?
    // 2. What about queries, the sequencer balance can be already queried via `Bank`
}
