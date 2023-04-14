pub mod hooks;

use sov_modules_api::Error;
use sov_modules_macros::ModuleInfo;
use sov_state::{StateValue, WorkingSet};

pub struct SequencerConfig<C: sov_modules_api::Context> {
    seq_rollup_address: C::Address,
    seq_da_address: Vec<u8>,
    coins_to_lock: StateValue<bank::Coins<C::Address>>,
}

#[derive(ModuleInfo)]
pub struct Sequencer<C: sov_modules_api::Context> {
    #[address]
    pub(crate) address: C::Address,

    #[module]
    pub(crate) bank: bank::Bank<C>,

    #[state]
    pub(crate) seq_rollup_address: StateValue<C::Address>,

    #[state]
    pub(crate) da_address: StateValue<Vec<u8>>,

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
        //Ok(self.init_module(config, working_set)?)
        todo!()
    }

    // Questions:
    // 1. There is no need to handle external calls?
    // 2. What about queries, the sequencer balance can be already queried via `Bank`
}
