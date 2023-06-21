pub mod call;
pub mod genesis;
pub mod hooks;
#[cfg(feature = "native")]
pub mod query;

use sov_modules_api::{CallResponse, Error, Spec};
use sov_modules_macros::ModuleInfo;
use sov_state::{StateMap, StateValue, WorkingSet};

/// Initial configuration for the sov_sequencer_registry module.
pub struct SequencerConfig<C: sov_modules_api::Context> {
    pub seq_rollup_address: C::Address,
    pub seq_da_address: Vec<u8>,
    pub coins_to_lock: sov_bank::Coins<C>,
}

#[derive(ModuleInfo)]
pub struct SequencerRegistry<C: sov_modules_api::Context> {
    /// The address of the sov_sequencer_registry module
    /// Note: this is address is generated by the module framework and the corresponding private key is unknown.
    #[address]
    pub(crate) address: C::Address,

    /// Reference to the Bank module.
    #[module]
    pub(crate) bank: sov_bank::Bank<C>,

    #[state]
    pub(crate) allowed_sequencers: StateMap<Vec<u8>, C::Address>,

    /// The sequencer address on the rollup.
    #[state]
    pub(crate) seq_rollup_address: StateValue<C::Address>,

    /// Coin's that will be slashed if the sequencer is malicious.
    /// The coins will be transferred from `self.seq_rollup_address` to `self.address`
    /// and locked forever.
    #[state]
    pub(crate) coins_to_lock: StateValue<sov_bank::Coins<C>>,
}

impl<C: sov_modules_api::Context> sov_modules_api::Module for SequencerRegistry<C> {
    type Context = C;

    type Config = SequencerConfig<C>;

    type CallMessage = call::CallMessage;

    fn genesis(
        &self,
        config: &Self::Config,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<(), Error> {
        Ok(self.init_module(config, working_set)?)
    }

    fn call(
        &self,
        message: Self::CallMessage,
        context: &Self::Context,
        working_set: &mut WorkingSet<<Self::Context as Spec>::Storage>,
    ) -> Result<CallResponse, Error> {
        Ok(match message {
            call::CallMessage::Register { da_address } => {
                self.register(da_address, context, working_set)?
            }
            call::CallMessage::Exit { da_address } => {
                self.exit(da_address, context, working_set)?
            }
        })
    }
}
