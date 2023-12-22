use anyhow::Result;
use serde::{Deserialize, Serialize};
use sov_modules_api::da::Time;
use sov_modules_api::{StateValueAccessor, WorkingSet};

use crate::{ChainState, TransitionHeight};

/// Initial configuration of the chain state
#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct ChainStateConfig {
    /// Initial slot height
    pub initial_slot_height: TransitionHeight,
    /// The time at genesis
    pub current_time: Time,
}

impl<C: sov_modules_api::Context, Da: sov_modules_api::DaSpec> ChainState<C, Da> {
    pub(crate) fn init_module(
        &self,
        config: &<Self as sov_modules_api::KernelModule>::Config,
        working_set: &mut WorkingSet<C>,
    ) -> Result<()> {
        self.genesis_height
            .set(&config.initial_slot_height, working_set);

        self.true_height
            .set(&config.initial_slot_height, working_set);

        self.time.set_genesis(&config.current_time, working_set);
        Ok(())
    }
}
