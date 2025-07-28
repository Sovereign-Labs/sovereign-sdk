use serde::{Deserialize, Serialize};
use sov_modules_api::{GenesisState, Module, Spec};

use crate::OperatorIncentives;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperatorIncentivesConfig<S: Spec> {
    pub reward_address: S::Address,
}

impl<S: Spec> OperatorIncentives<S> {
    pub(crate) fn init_module(
        &mut self,
        config: &<Self as Module>::Config,
        state: &mut impl GenesisState<S>,
    ) -> anyhow::Result<()> {
        self.reward_address.set(&config.reward_address, state)?;
        Ok(())
    }
}
