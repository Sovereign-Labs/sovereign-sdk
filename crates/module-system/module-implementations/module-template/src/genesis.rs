use anyhow::Result;
use sov_modules_api::{GenesisState, Module, Spec};

use crate::ExampleModule;

impl<S: Spec> ExampleModule<S> {
    pub(crate) fn init_module(
        &self,
        _config: &<Self as Module>::Config,
        _state: &mut impl GenesisState<S>,
    ) -> Result<()> {
        Ok(())
    }
}
