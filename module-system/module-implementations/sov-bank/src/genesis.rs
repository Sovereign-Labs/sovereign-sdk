use anyhow::{bail, Result};
use sov_state::WorkingSet;

use crate::token::Token;
use crate::Bank;

pub(crate) const DEPLOYER: [u8; 32] = [0; 32];

impl<C: sov_modules_api::Context> Bank<C> {
    pub(crate) fn init_module(
        &self,
        config: &<Self as sov_modules_api::Module>::Config,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<()> {
        let parent_prefix = self.tokens.prefix();
        for token_config in config.tokens.iter() {
            let (token_address, token) = Token::<C>::create(
                &token_config.token_name,
                &token_config.address_and_balances,
                &token_config.authorized_minters,
                &DEPLOYER,
                token_config.salt,
                parent_prefix,
                working_set,
            )?;

            if self.tokens.get(&token_address, working_set).is_some() {
                bail!("Token address {} already exists", token_address);
            }

            self.tokens.set(&token_address, &token, working_set);
        }
        Ok(())
    }
}
