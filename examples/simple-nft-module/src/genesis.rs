use anyhow::{bail, Result};
use sov_modules_api::Context;
use sov_state::WorkingSet;

use crate::NonFungibleToken;

impl<C: Context> NonFungibleToken<C> {
    pub(crate) fn init_module(
        &self,
        config: &<Self as sov_modules_api::Module>::Config,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<()> {
        self.admin.set(&config.admin, working_set);
        for (id, owner) in config.owners.iter() {
            if self.owners.get(id, working_set).is_some() {
                bail!("Token id {} already exists", id);
            }
            self.owners.set(id, owner, working_set);
        }
        Ok(())
    }
}
